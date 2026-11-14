//! XI.8.3: vertex/index/uniform buffer → Metal resource, mock geometry triangle render fixture.

use stellar_renderer_core::backend::{
    BackendError, BackendErrorKind, BlendState, BufferDesc, BufferHandle, BufferUsage,
    CallOrderState, CheckedBackend, DepthMode, DrawCommand, EncodeOutcome, PassCommands,
    PipelineKey, RendererBackend, ScissorRect, ShaderProgram, StencilMode, TextureHandle,
    TextureRegion,
};
use stellar_renderer_core::render_graph::{
    AttachmentSize, ClearColor, CompiledRenderGraph, ImportKind, LoadOp, PassId, PassKind,
    RenderGraphBuilder, SampleCount, StoreOp, TextureDesc, TextureFormat,
};

use super::*;
use crate::MetalBackend;

const SIZE: u32 = 16;
const GREEN: [u8; 4] = [0, 255, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// `SolidFill` vertex 배열(`{ float2 position; float4 color; }`, little-endian f32).
pub(crate) fn solid_vertices(points: &[([f32; 2], [f32; 4])]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(points.len() * 24);
    for (position, color) in points {
        for value in position.iter().chain(color.iter()) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

/// `DrawUniforms { float4 tint; float depth; float pad[3]; }`.
pub(crate) fn draw_uniforms(tint: [f32; 4], depth: f32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(32);
    for value in tint.iter().chain([depth, 0.0, 0.0, 0.0].iter()) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// 사각형 두 삼각형(6 vertex) `SolidFill` vertex.
pub(crate) fn quad(x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]) -> Vec<u8> {
    solid_vertices(&[
        ([x0, y0], color),
        ([x1, y0], color),
        ([x0, y1], color),
        ([x0, y1], color),
        ([x1, y0], color),
        ([x1, y1], color),
    ])
}

pub(crate) fn solid_key(format: TextureFormat, blend: BlendState) -> PipelineKey {
    PipelineKey {
        program: ShaderProgram::SolidFill,
        blend,
        depth: DepthMode::Disabled,
        stencil: StencilMode::Disabled,
        color_format: format,
        samples: SampleCount::X1,
    }
}

/// external RGBA8 target 하나에 그리는 headless 환경.
pub(crate) struct Offscreen {
    pub(crate) backend: CheckedBackend<MetalBackend>,
    pub(crate) target: TextureHandle,
    next_frame: u64,
}

impl Offscreen {
    pub(crate) fn new(width: u32, height: u32) -> Offscreen {
        let mut backend = CheckedBackend::new(MetalBackend::headless());
        backend
            .init(&device_config(width, height, TextureFormat::Bgra8Unorm))
            .expect("init");
        let target = backend
            .create_texture(&target_desc(width, height, TextureFormat::Rgba8Unorm))
            .expect("target");
        Offscreen {
            backend,
            target,
            next_frame: 1,
        }
    }

    pub(crate) fn buffer(&mut self, usage: BufferUsage, data: &[u8]) -> BufferHandle {
        let handle = self
            .backend
            .create_buffer(&BufferDesc {
                size: data.len() as u64,
                usage,
            })
            .expect("buffer");
        self.backend.update_buffer(handle, 0, data).expect("update");
        handle
    }

    /// pass 하나짜리 graph를 encode합니다. 실패하면 오류를 돌려주고 frame은 열리지 않습니다.
    pub(crate) fn try_render(
        &mut self,
        graph: &CompiledRenderGraph,
        commands: Vec<DrawCommand>,
    ) -> Result<Vec<u8>, BackendError> {
        let imports = bind_first_import(graph, self.target);
        let passes = [PassCommands {
            pass: PassId(0),
            commands,
        }];
        let frame = self.next_frame;
        self.next_frame += 1;
        let request = stellar_renderer_core::backend::FrameEncodeRequest {
            frame: stellar_renderer_core::backend::FrameId(frame),
            graph,
            passes: &passes,
            imports: &imports,
        };
        let EncodeOutcome::Encoded(encoded) = self.backend.encode(&request)? else {
            panic!("offscreen frame은 skip하지 않습니다");
        };
        let submission = self.backend.submit(encoded)?;
        let progress = self.backend.inner_mut().wait_idle()?;
        assert_eq!(progress.completed, Some(submission));
        self.backend.inner_mut().read_texture(self.target)
    }

    pub(crate) fn render(
        &mut self,
        graph: &CompiledRenderGraph,
        commands: Vec<DrawCommand>,
    ) -> Vec<u8> {
        self.try_render(graph, commands).expect("render")
    }
}

/// target을 `clear`(또는 Load)로 시작하는 pass 하나 graph.
pub(crate) fn single_pass(size: u32, load: LoadOp, clear: ClearColor) -> CompiledRenderGraph {
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import(
        "target",
        TextureDesc {
            format: TextureFormat::Rgba8Unorm,
            size: AttachmentSize::Fixed {
                width: size,
                height: size,
            },
            samples: SampleCount::X1,
        },
        ImportKind::External,
    );
    let pass = builder.pass("draw", PassKind::Raster);
    let written = builder
        .color(pass, target, load, clear, StoreOp::Store)
        .expect("color");
    builder.set_output(written);
    builder.finish().compile().expect("compile")
}

pub(crate) fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * width + x) * 4) as usize;
    [
        pixels[offset],
        pixels[offset + 1],
        pixels[offset + 2],
        pixels[offset + 3],
    ]
}

fn black() -> ClearColor {
    unit_clear([false, false, false, true])
}

/// 꼭짓점 `a → b` 직선까지 부호 있는 거리(삼각형 안쪽이 양수가 되도록 `sign`을 곱함).
fn edge_distance(a: [f32; 2], b: [f32; 2], p: [f32; 2], sign: f32) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    sign * (dx * (p[1] - a[1]) - dy * (p[0] - a[0])) / (dx * dx + dy * dy).sqrt()
}

#[test]
fn mock_geometry_triangle_renders_inside_pixels_only() {
    // 검증 기준(XI.8.3): mock geometry triangle render fixture 통과.
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(SIZE, SIZE);
    let corners = [[8.0, 1.0], [15.0, 15.0], [1.0, 15.0]];
    let green = [0.0, 1.0, 0.0, 1.0];
    let vertices = env.buffer(
        BufferUsage::Vertex,
        &solid_vertices(&[
            (corners[0], green),
            (corners[1], green),
            (corners[2], green),
        ]),
    );
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let graph = single_pass(SIZE, LoadOp::Clear, black());
    let pixels = env.render(
        &graph,
        vec![
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
                vertex_count: 3,
                instance_count: 1,
            },
        ],
    );
    // 화면 좌표(y 아래)에서 이 꼭짓점 순서의 안쪽은 edge 거리 부호가 양수 → sign +1.
    let (mut inside, mut outside) = (0, 0);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let center = [x as f32 + 0.5, y as f32 + 0.5];
            let margin = (0..3)
                .map(|i| edge_distance(corners[i], corners[(i + 1) % 3], center, 1.0))
                .fold(f32::INFINITY, f32::min);
            let actual = pixel(&pixels, SIZE, x, y);
            if margin >= 0.75 {
                assert_eq!(actual, GREEN, "안쪽 ({x}, {y})");
                inside += 1;
            } else if margin <= -0.75 {
                assert_eq!(actual, BLACK, "바깥 ({x}, {y})");
                outside += 1;
            }
        }
    }
    assert!(
        inside >= 40 && outside >= 60,
        "inside {inside}, outside {outside}"
    );
}

#[test]
fn indexed_quad_with_base_vertex_and_uniform_tint() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(SIZE, SIZE);
    let red = [1.0, 0.0, 0.0, 1.0];
    // 앞의 vertex 2개는 base_vertex로 건너뜁니다.
    let vertices = env.buffer(
        BufferUsage::Vertex,
        &solid_vertices(&[
            ([0.0, 0.0], [0.0; 4]),
            ([0.0, 0.0], [0.0; 4]),
            ([4.0, 4.0], red),
            ([12.0, 4.0], red),
            ([4.0, 12.0], red),
            ([12.0, 12.0], red),
        ]),
    );
    let indices: Vec<u8> = [0u32, 1, 2, 2, 1, 3]
        .iter()
        .flat_map(|index| index.to_le_bytes())
        .collect();
    let indices = env.buffer(BufferUsage::Index, &indices);
    // uniform 두 개를 16-byte 정렬 offset에 둡니다: 0 = 그대로, 32 = 절반 tint.
    let mut uniform_bytes = draw_uniforms([1.0; 4], 0.0);
    uniform_bytes.extend(draw_uniforms([0.5; 4], 0.0));
    let uniforms = env.buffer(BufferUsage::Uniform, &uniform_bytes);
    let graph = single_pass(SIZE, LoadOp::Clear, black());
    let draw = |uniform_offset| {
        vec![
            DrawCommand::SetPipeline(solid_key(TextureFormat::Rgba8Unorm, BlendState::Replace)),
            DrawCommand::SetVertexBuffer {
                buffer: vertices,
                offset: 0,
            },
            DrawCommand::SetIndexBuffer {
                buffer: indices,
                offset: 0,
            },
            DrawCommand::SetUniforms {
                buffer: uniforms,
                offset: uniform_offset,
                size: 32,
            },
            DrawCommand::DrawIndexed {
                index_start: 0,
                index_count: 6,
                base_vertex: 2,
                instance_count: 1,
            },
        ]
    };
    let full = env.render(&graph, draw(0));
    for y in 0..SIZE {
        for x in 0..SIZE {
            let inside = (4..12).contains(&x) && (4..12).contains(&y);
            let expected = if inside { RED } else { BLACK };
            assert_eq!(pixel(&full, SIZE, x, y), expected, "({x}, {y})");
        }
    }
    // tint 0.5: premultiplied (0.5, 0, 0, 0.5)를 Replace로 씁니다(unorm 반올림 127 또는 128).
    let half = env.render(&graph, draw(32));
    let [r, g, b, a] = pixel(&half, SIZE, 8, 8);
    assert!((127..=128).contains(&r) && g == 0 && b == 0 && (127..=128).contains(&a));
    assert_eq!(pixel(&half, SIZE, 0, 0), BLACK);
    // 같은 key의 pipeline은 frame마다 다시 만들지 않습니다.
    assert_eq!(env.backend.inner().pipeline_count(), 1);
}

#[test]
fn scissor_clamps_to_attachment_and_empty_scissor_skips_draws() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(SIZE, SIZE);
    let vertices = env.buffer(
        BufferUsage::Vertex,
        &quad(0.0, 0.0, 16.0, 16.0, [0.0, 1.0, 0.0, 1.0]),
    );
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let graph = single_pass(SIZE, LoadOp::Clear, black());
    let draw = |scissor| {
        vec![
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
            DrawCommand::SetScissor(scissor),
            DrawCommand::Draw {
                vertex_start: 0,
                vertex_count: 6,
                instance_count: 1,
            },
        ]
    };
    let inside = |pixels: &[u8], rect: (u32, u32, u32, u32)| {
        for y in 0..SIZE {
            for x in 0..SIZE {
                let hit = (rect.0..rect.0 + rect.2).contains(&x)
                    && (rect.1..rect.1 + rect.3).contains(&y);
                let expected = if hit { GREEN } else { BLACK };
                assert_eq!(pixel(pixels, SIZE, x, y), expected, "({x}, {y}) {rect:?}");
            }
        }
    };
    let rect = env.render(
        &graph,
        draw(Some(ScissorRect {
            x: 2,
            y: 3,
            width: 5,
            height: 4,
        })),
    );
    inside(&rect, (2, 3, 5, 4));
    let clamped = env.render(
        &graph,
        draw(Some(ScissorRect {
            x: 12,
            y: 10,
            width: 100,
            height: u32::MAX,
        })),
    );
    inside(&clamped, (12, 10, 4, 6));
    let empty = env.render(
        &graph,
        draw(Some(ScissorRect {
            x: 20,
            y: 20,
            width: 4,
            height: 4,
        })),
    );
    assert!(all_pixels(&empty, BLACK));
    let full = env.render(&graph, draw(None));
    assert!(all_pixels(&full, GREEN));
}

#[test]
fn premultiplied_over_blends_with_destination() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(SIZE, SIZE);
    let vertices = env.buffer(
        BufferUsage::Vertex,
        &quad(0.0, 0.0, 16.0, 16.0, [0.5, 0.0, 0.0, 0.5]),
    );
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let graph = single_pass(SIZE, LoadOp::Clear, unit_clear([false, false, true, true]));
    let pixels = env.render(
        &graph,
        vec![
            DrawCommand::SetPipeline(solid_key(
                TextureFormat::Rgba8Unorm,
                BlendState::PremultipliedOver,
            )),
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
        ],
    );
    // src + dst × (1 − 0.5) = (0.5, 0, 0.5, 1).
    let [r, g, b, a] = pixel(&pixels, SIZE, 5, 5);
    assert!((127..=128).contains(&r) && g == 0 && (127..=128).contains(&b) && a == 255);
}

#[test]
fn texture_upload_is_queue_ordered_before_the_next_frame() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(4, 4);
    // RGBA8 4×4 pattern: pixel (x, y) = (x*60, y*60, 7, 255).
    let pattern: Vec<u8> = (0..4u8)
        .flat_map(|y| (0..4u8).flat_map(move |x| [x * 60, y * 60, 7, 255]))
        .collect();
    let full = TextureRegion {
        mip_level: 0,
        x: 0,
        y: 0,
        width: 4,
        height: 4,
    };
    env.backend
        .update_texture(env.target, full, &pattern)
        .expect("upload");
    // 부분 갱신: (1, 2)부터 2×1.
    let patch = [9, 9, 9, 9, 8, 8, 8, 8];
    env.backend
        .update_texture(
            env.target,
            TextureRegion {
                mip_level: 0,
                x: 1,
                y: 2,
                width: 2,
                height: 1,
            },
            &patch,
        )
        .expect("patch");
    let mut expected = pattern.clone();
    expected[(2 * 4 + 1) * 4..(2 * 4 + 3) * 4].copy_from_slice(&patch);
    assert_eq!(
        env.backend.inner_mut().read_texture(env.target).unwrap(),
        expected
    );
    // upload 뒤 LoadOp::Load frame은 upload 결과 위에 그립니다(queue 순서).
    env.backend
        .update_texture(env.target, full, &pattern)
        .expect("upload again");
    let vertices = env.buffer(
        BufferUsage::Vertex,
        &quad(0.0, 0.0, 1.0, 1.0, [0.0, 1.0, 0.0, 1.0]),
    );
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let graph = single_pass(4, LoadOp::Load, black());
    let pixels = env.render(
        &graph,
        vec![
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
        ],
    );
    let mut expected = pattern;
    expected[0..4].copy_from_slice(&GREEN);
    assert_eq!(pixels, expected);
}

#[test]
fn texture_upload_regions_are_validated() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(4, 4);
    let mipped = env
        .backend
        .create_texture(&stellar_renderer_core::backend::GpuTextureDesc {
            mip_levels: 3,
            ..target_desc(4, 4, TextureFormat::Rgba8Unorm)
        })
        .expect("mipped");
    let region = |mip_level, x, y, width, height| TextureRegion {
        mip_level,
        x,
        y,
        width,
        height,
    };
    // mip 2는 1×1입니다.
    assert!(env
        .backend
        .update_texture(mipped, region(2, 0, 0, 1, 1), &[1, 2, 3, 4])
        .is_ok());
    for (bad, data) in [
        (region(3, 0, 0, 1, 1), vec![0; 4]),
        (region(1, 1, 0, 2, 1), vec![0; 8]),
        (region(0, 0, 0, 0, 1), vec![]),
        (region(0, u32::MAX, 0, 2, 1), vec![0; 8]),
        (region(0, 0, 0, 2, 2), vec![0; 15]),
    ] {
        assert_eq!(
            env.backend
                .update_texture(mipped, bad, &data)
                .unwrap_err()
                .kind(),
            BackendErrorKind::InvalidUsage,
            "{bad:?}"
        );
    }
    env.backend.inner_mut().wait_idle().expect("wait");
}

#[test]
fn buffers_map_usage_bounds_and_lifetime() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(4, 4);
    let buffer = env.buffer(BufferUsage::Vertex, &[0; 48]);
    assert!(env.backend.update_buffer(buffer, 40, &[1; 8]).is_ok());
    assert!(env.backend.update_buffer(buffer, 48, &[]).is_ok());
    for (offset, len) in [(41, 8), (49, 0), (u64::MAX, 1)] {
        assert_eq!(
            env.backend
                .update_buffer(buffer, offset, &vec![0; len])
                .unwrap_err()
                .kind(),
            BackendErrorKind::InvalidUsage
        );
    }
    assert_eq!(
        env.backend
            .create_buffer(&BufferDesc {
                size: 0,
                usage: BufferUsage::Staging
            })
            .unwrap_err()
            .kind(),
        BackendErrorKind::InvalidUsage
    );
    // device 상한을 넘는 크기는 할당을 시도하지 않고 메모리 부족으로 분류합니다.
    assert_eq!(
        env.backend
            .create_buffer(&BufferDesc {
                size: u64::MAX,
                usage: BufferUsage::Vertex
            })
            .unwrap_err()
            .kind(),
        BackendErrorKind::OutOfDeviceMemory
    );
    env.backend.destroy_buffer(buffer).expect("destroy");
    assert_eq!(
        env.backend
            .update_buffer(buffer, 0, &[1])
            .unwrap_err()
            .kind(),
        BackendErrorKind::InvalidUsage
    );
}

#[test]
fn invalid_draw_commands_never_reach_the_gpu() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(SIZE, SIZE);
    let vertices = env.buffer(
        BufferUsage::Vertex,
        &quad(0.0, 0.0, 16.0, 16.0, [1.0, 1.0, 1.0, 1.0]),
    );
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let far_index: Vec<u8> = [0u32, 1, 100]
        .iter()
        .flat_map(|i| i.to_le_bytes())
        .collect();
    let indices = env.buffer(BufferUsage::Index, &far_index);
    let graph = single_pass(SIZE, LoadOp::Clear, black());
    let pipeline =
        DrawCommand::SetPipeline(solid_key(TextureFormat::Rgba8Unorm, BlendState::Replace));
    let bind_vertices = DrawCommand::SetVertexBuffer {
        buffer: vertices,
        offset: 0,
    };
    let bind_uniforms = DrawCommand::SetUniforms {
        buffer: uniforms,
        offset: 0,
        size: 32,
    };
    let draw = |start, count| DrawCommand::Draw {
        vertex_start: start,
        vertex_count: count,
        instance_count: 1,
    };
    let cases: Vec<(Vec<DrawCommand>, BackendErrorKind)> = vec![
        // pipeline 없이 draw.
        (
            vec![bind_vertices, bind_uniforms, draw(0, 3)],
            BackendErrorKind::InvalidUsage,
        ),
        // uniform 없이 draw.
        (
            vec![pipeline, bind_vertices, draw(0, 3)],
            BackendErrorKind::InvalidUsage,
        ),
        // vertex 범위 밖(6개 buffer에 7개).
        (
            vec![pipeline, bind_vertices, bind_uniforms, draw(0, 7)],
            BackendErrorKind::InvalidUsage,
        ),
        (
            vec![pipeline, bind_vertices, bind_uniforms, draw(u32::MAX, 2)],
            BackendErrorKind::InvalidUsage,
        ),
        // index 값 100이 vertex 용량 밖.
        (
            vec![
                pipeline,
                bind_vertices,
                bind_uniforms,
                DrawCommand::SetIndexBuffer {
                    buffer: indices,
                    offset: 0,
                },
                DrawCommand::DrawIndexed {
                    index_start: 0,
                    index_count: 3,
                    base_vertex: 0,
                    instance_count: 1,
                },
            ],
            BackendErrorKind::InvalidUsage,
        ),
        // index 범위가 index buffer 밖, 음수 base vertex.
        (
            vec![
                pipeline,
                bind_vertices,
                bind_uniforms,
                DrawCommand::SetIndexBuffer {
                    buffer: indices,
                    offset: 0,
                },
                DrawCommand::DrawIndexed {
                    index_start: 1,
                    index_count: 3,
                    base_vertex: 0,
                    instance_count: 1,
                },
            ],
            BackendErrorKind::InvalidUsage,
        ),
        (
            vec![
                pipeline,
                bind_vertices,
                bind_uniforms,
                DrawCommand::SetIndexBuffer {
                    buffer: indices,
                    offset: 0,
                },
                DrawCommand::DrawIndexed {
                    index_start: 0,
                    index_count: 2,
                    base_vertex: -1,
                    instance_count: 1,
                },
            ],
            BackendErrorKind::InvalidUsage,
        ),
        // 용도가 다른 buffer, 정렬되지 않은 uniform offset, 작은 uniform 범위.
        (
            vec![DrawCommand::SetUniforms {
                buffer: vertices,
                offset: 0,
                size: 32,
            }],
            BackendErrorKind::InvalidUsage,
        ),
        (
            vec![DrawCommand::SetUniforms {
                buffer: uniforms,
                offset: 8,
                size: 24,
            }],
            BackendErrorKind::InvalidUsage,
        ),
        (
            vec![DrawCommand::SetUniforms {
                buffer: uniforms,
                offset: 0,
                size: 16,
            }],
            BackendErrorKind::InvalidUsage,
        ),
        (
            vec![DrawCommand::SetVertexBuffer {
                buffer: vertices,
                offset: 2,
            }],
            BackendErrorKind::InvalidUsage,
        ),
        // pass와 다른 color format.
        (
            vec![DrawCommand::SetPipeline(solid_key(
                TextureFormat::Bgra8Unorm,
                BlendState::Replace,
            ))],
            BackendErrorKind::InvalidUsage,
        ),
    ];
    for (commands, kind) in cases {
        let error = env.try_render(&graph, commands.clone()).unwrap_err();
        assert_eq!(error.kind(), kind, "{commands:?}");
        assert_eq!(env.backend.validator().state(), CallOrderState::Idle);
    }
    let progress = env.backend.poll().expect("poll");
    assert_eq!(progress.completed, None, "거부한 frame은 제출되지 않습니다");
    // 같은 환경에서 올바른 frame은 그대로 그립니다.
    let pixels = env.render(
        &graph,
        vec![pipeline, bind_vertices, bind_uniforms, draw(0, 6)],
    );
    assert!(all_pixels(&pixels, [255; 4]));
}

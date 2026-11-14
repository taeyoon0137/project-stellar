//! XI.8.6: Metal GPU timestamp query와 pass별 GPU 시간.

use stellar_renderer_core::backend::{
    BlendState, BufferDesc, BufferHandle, BufferUsage, CheckedBackend, DeviceConfig, DrawCommand,
    EncodeOutcome, FrameEncodeRequest, FrameId, PassCommands, RendererBackend, ShaderProgram,
    SubmissionId, TextureHandle, TextureSource, TimestampQuery,
};
use stellar_renderer_core::placeholder::PLACEHOLDER_SAMPLER;
use stellar_renderer_core::render_graph::{
    AttachmentSize, CompiledRenderGraph, ImportKind, LoadOp, PassId, PassKind, RenderGraphBuilder,
    SampleCount, StoreOp, TextureDesc, TextureFormat,
};

use super::geometry::{draw_uniforms, quad, solid_key};
use super::shaders::{key, textured_quad};
use super::*;
use crate::{gpu_timing_line, MetalBackend, MAX_TIMING_RESULTS};

const SIZE: u32 = 256;

fn timed_config(timestamps: bool) -> DeviceConfig {
    DeviceConfig {
        timestamps,
        ..device_config(SIZE, SIZE, TextureFormat::Bgra8Unorm)
    }
}

/// layer pass(큰 quad 여러 장) → composite pass로 이루어진 "simple scene".
struct Scene {
    backend: CheckedBackend<MetalBackend>,
    graph: CompiledRenderGraph,
    target: TextureHandle,
    passes: Vec<PassCommands>,
    next_frame: u64,
}

fn buffer(
    backend: &mut CheckedBackend<MetalBackend>,
    usage: BufferUsage,
    data: &[u8],
) -> BufferHandle {
    let handle = backend
        .create_buffer(&BufferDesc {
            size: data.len() as u64,
            usage,
        })
        .expect("buffer");
    backend.update_buffer(handle, 0, data).expect("update");
    handle
}

impl Scene {
    fn new(timestamps: bool) -> Scene {
        let mut backend = CheckedBackend::new(MetalBackend::headless());
        backend.init(&timed_config(timestamps)).expect("init");
        let fixed = TextureDesc {
            format: TextureFormat::Rgba8Unorm,
            size: AttachmentSize::Fixed {
                width: SIZE,
                height: SIZE,
            },
            samples: SampleCount::X1,
        };
        let mut builder = RenderGraphBuilder::new();
        let target = builder.import("target", fixed, ImportKind::External);
        let layer = builder.transient("layer", fixed);
        let paint = builder.pass("layer", PassKind::Raster);
        let layer_v1 = builder
            .color(
                paint,
                layer,
                LoadOp::Clear,
                unit_clear([false; 4]),
                StoreOp::Store,
            )
            .expect("layer");
        let composite = builder.pass("composite", PassKind::Raster);
        builder.read(composite, layer_v1).expect("read");
        let written = builder
            .color(
                composite,
                target,
                LoadOp::Clear,
                unit_clear([false, false, false, true]),
                StoreOp::Store,
            )
            .expect("target");
        builder.set_output(written);
        let graph = builder.finish().compile().expect("compile");
        let target_texture = backend
            .create_texture(&target_desc(SIZE, SIZE, TextureFormat::Rgba8Unorm))
            .expect("target");
        let side = SIZE as f32;
        // 반투명 full-screen quad 32장(overdraw)으로 측정 가능한 GPU 작업을 만듭니다.
        let mut solid = Vec::new();
        for _ in 0..32 {
            solid.extend(quad(0.0, 0.0, side, side, [0.02, 0.01, 0.0, 0.02]));
        }
        let solid = buffer(&mut backend, BufferUsage::Vertex, &solid);
        let textured = buffer(
            &mut backend,
            BufferUsage::Vertex,
            &textured_quad(side, side, 1.0, 1.0),
        );
        let uniforms = buffer(
            &mut backend,
            BufferUsage::Uniform,
            &draw_uniforms([1.0; 4], 0.0),
        );
        let passes = vec![
            PassCommands {
                pass: PassId(0),
                commands: vec![
                    DrawCommand::SetPipeline(solid_key(
                        TextureFormat::Rgba8Unorm,
                        BlendState::PremultipliedOver,
                    )),
                    DrawCommand::SetVertexBuffer {
                        buffer: solid,
                        offset: 0,
                    },
                    DrawCommand::SetUniforms {
                        buffer: uniforms,
                        offset: 0,
                        size: 32,
                    },
                    DrawCommand::Draw {
                        vertex_start: 0,
                        vertex_count: 6 * 32,
                        instance_count: 1,
                    },
                ],
            },
            PassCommands {
                pass: PassId(1),
                commands: vec![
                    DrawCommand::SetPipeline(key(
                        ShaderProgram::LayerComposite,
                        TextureFormat::Rgba8Unorm,
                    )),
                    DrawCommand::SetVertexBuffer {
                        buffer: textured,
                        offset: 0,
                    },
                    DrawCommand::SetUniforms {
                        buffer: uniforms,
                        offset: 0,
                        size: 32,
                    },
                    DrawCommand::BindTexture {
                        slot: 0,
                        texture: TextureSource::Graph(layer_v1),
                        sampler: PLACEHOLDER_SAMPLER,
                    },
                    DrawCommand::Draw {
                        vertex_start: 0,
                        vertex_count: 6,
                        instance_count: 1,
                    },
                ],
            },
        ];
        Scene {
            backend,
            graph,
            target: target_texture,
            passes,
            next_frame: 1,
        }
    }

    fn submit(&mut self) -> SubmissionId {
        let imports = bind_first_import(&self.graph, self.target);
        let request = FrameEncodeRequest {
            frame: FrameId(self.next_frame),
            graph: &self.graph,
            passes: &self.passes,
            imports: &imports,
        };
        self.next_frame += 1;
        let EncodeOutcome::Encoded(frame) = self.backend.encode(&request).expect("encode") else {
            panic!("encode");
        };
        self.backend.submit(frame).expect("submit")
    }
}

#[test]
fn simple_scene_reports_gpu_duration_per_pass() {
    // 검증 기준(XI.8.6): simple scene GPU duration report.
    if !metal_available() {
        return;
    }
    let mut scene = Scene::new(true);
    let caps = scene.backend.inner().capabilities().expect("caps");
    if !caps.timestamp_query {
        eprintln!("이 GPU는 stage boundary timestamp를 지원하지 않습니다: 검증 없이 건너뜁니다");
        return;
    }
    let submission = scene.submit();
    // 제출 직후에는 아직 끝나지 않았거나(Pending) 이미 끝났습니다(Ready).
    let early = scene.backend.query_timestamps(submission).expect("query");
    assert!(matches!(
        early,
        TimestampQuery::Pending | TimestampQuery::Ready(_)
    ));
    scene.backend.inner_mut().wait_idle().expect("wait");
    let TimestampQuery::Ready(timings) = scene.backend.query_timestamps(submission).expect("query")
    else {
        panic!("GPU가 끝난 submission은 Ready여야 합니다");
    };
    eprintln!("{}", gpu_timing_line(submission, &timings));
    // pass는 graph 실행 순서이고, 그린 pass마다 0보다 큰 GPU 시간을 보고합니다.
    let order: Vec<PassId> = timings.passes.iter().map(|pass| pass.pass).collect();
    assert_eq!(order, scene.graph.order());
    assert!(
        timings.passes.iter().all(|pass| pass.nanos > 0),
        "{timings:?}"
    );
    assert!(timings.total_nanos > 0);
    // 비현실적인 값(1초 이상)이 아니어야 합니다(tick → ns 환산 확인).
    assert!(timings.total_nanos < 1_000_000_000, "{timings:?}");
    assert!(
        timings.passes.iter().all(|pass| pass.nanos < 1_000_000_000),
        "{timings:?}"
    );
}

#[test]
fn frames_in_flight_reuse_sample_buffers_and_keep_recent_results() {
    if !metal_available() {
        return;
    }
    let mut scene = Scene::new(true);
    if !scene
        .backend
        .inner()
        .capabilities()
        .is_some_and(|caps| caps.timestamp_query)
    {
        return;
    }
    // 3 frame을 기다리지 않고 제출한 뒤 한꺼번에 기다립니다.
    let batch: Vec<SubmissionId> = (0..3).map(|_| scene.submit()).collect();
    scene.backend.inner_mut().wait_idle().expect("wait");
    for submission in &batch {
        assert!(matches!(
            scene.backend.query_timestamps(*submission).unwrap(),
            TimestampQuery::Ready(_)
        ));
    }
    // 보관 상한을 넘기면 가장 오래된 결과부터 Unsupported(보관 범위 밖)입니다.
    let mut last = batch[2];
    for _ in 0..MAX_TIMING_RESULTS {
        last = scene.submit();
        scene.backend.inner_mut().wait_idle().expect("wait");
    }
    assert_eq!(
        scene.backend.query_timestamps(batch[0]).unwrap(),
        TimestampQuery::Unsupported
    );
    assert!(matches!(
        scene.backend.query_timestamps(last).unwrap(),
        TimestampQuery::Ready(_)
    ));
}

#[test]
fn timestamps_not_requested_are_unsupported() {
    if !metal_available() {
        return;
    }
    let mut scene = Scene::new(false);
    let submission = scene.submit();
    scene.backend.inner_mut().wait_idle().expect("wait");
    assert_eq!(
        scene.backend.query_timestamps(submission).unwrap(),
        TimestampQuery::Unsupported
    );
}

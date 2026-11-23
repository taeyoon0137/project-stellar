//! 2만 가시 node scene 구간 benchmark: scene build → visibility → render graph → encode → submit (XI.11.4, ADR-0102 결정 7).
//!
//! 1920×1080 viewport를 card 2,000개(Frame 1 + Shape·Path·Text·Image 자식 9 = 10 node)로 채워 20,000 node가 가시입니다(XI.3.6과
//! 같은 card 구성). 숨김 card 100개와 viewport 밖 card 100개를 함께 둡니다. frame마다 다음 단계를 재고 nearest-rank
//! p50/p95/p99 JSON을 냅니다.
//!
//! | 단계 | 범위 |
//! |---|---|
//! | `scene_build` | `SceneBuilder`로 snapshot 전체 생성(최악 경우: 모든 node 재빌드) |
//! | `visibility` | 공간 index build + viewport 가시 후보(XI.3.1~XI.3.2) |
//! | `render_graph` | `compile_frame`(pass·layer·clip·graph compile, XI.4.2~XI.4.4) + `batch_frame`(XI.4.5) |
//! | `encode` | batch → draw command·정점·uniform 생성, buffer 갱신, Metal `encode`(XI.8.5) |
//! | `submit` | `FrameScheduler::tick`의 encode 밖 부분: poll, present 예약, Metal `submit`(XI.5.5, XI.8.5) |
//!
//! GPU 완료는 frame마다 기다려(측정 밖) present queue를 비웁니다. 결과 JSON에는 XI.11.5 present queue 지표(`present_queue`)와
//! 아키텍처 §6.2 단계 예산(µs, 참고)을 넣습니다.
//!
//! **batch → draw command 변환은 benchmark 대리 구현입니다.** 제품 frame encoder가 아직 없어 draw마다 node world bounds 사각형
//! 하나(단색은 첫 fill 색, texture program은 1×1 대체 texture)를 그립니다. 제품 frame encoder가 생기면 이 단계를 그것으로 바꿉니다.
//!
//! - 측정 실행(release, 결과 JSON 한 줄 출력):
//!   `cargo test --release -p stellar-bench-fixtures --test render_stage_benchmark -- --ignored --nocapture`
//! - `STELLAR_RENDER_BENCH_OUT=<path>`를 주면 같은 JSON을 그 파일에도 씁니다.
//! - 기본 `cargo test`는 같은 scene에서 frame 몇 개로 JSON 구성·집계·queue 깊이만 확인합니다(시간 판정 없음).
//! - Metal device가 없으면 실패합니다. `STELLAR_ALLOW_MISSING_METAL=1`일 때만 이유를 출력하고 건너뜁니다.
#![cfg(target_os = "macos")]

use std::fmt::Write as _;
use std::time::Instant;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::revision::{DocumentRevision, SceneRevision};
use stellar_core_types::shadow::BlurRadius;
use stellar_core_types::value::{Rect, Transform2D};
use stellar_renderer_core::backend::{
    BackendError, BufferDesc, BufferHandle, BufferUsage, CheckedBackend, DeviceConfig, DrawCommand,
    EncodeOutcome, FrameEncodeRequest, FrameId, GpuTextureDesc, ImportBinding, ImportTarget,
    PassCommands, RendererBackend, ShaderProgram, SurfaceConfig, SurfaceSize, TextureHandle,
    TextureRegion, TextureSource, TextureUsage,
};
use stellar_renderer_core::batching::{
    batch_count, batch_frame, ClipKey, PassBatches, TargetInfo, TextureKey,
};
use stellar_renderer_core::clip_plan::ClipCapabilities;
use stellar_renderer_core::color::to_working;
use stellar_renderer_core::effects::abi::{
    draw_uniforms, push_solid_vertex, push_textured_vertex, DRAW_UNIFORMS_SIZE, MODE_PLAIN,
    UNIFORM_ALIGNMENT,
};
use stellar_renderer_core::frame_scheduler::{FrameScheduler, TickOutcome};
use stellar_renderer_core::output_color::{OutputColor, OutputColorPlan};
use stellar_renderer_core::pass_compiler::{
    compile_frame, CompileInput, EffectBoundaries, FramePlan,
};
use stellar_renderer_core::placeholder::PLACEHOLDER_SAMPLER;
use stellar_renderer_core::present_metrics::PresentRecorder;
use stellar_renderer_core::render_graph::{ClearColor, SampleCount, TextureFormat};
use stellar_renderer_metal::{LayerOptions, MetalBackend};
use stellar_scene::builder::{
    FrameClipInput, FrameInput, ImageInput, ModifierPaint, NodeInput, PathInput, SceneBuilder,
    SceneScope, ShapeInput, StrokeInput, TextInput,
};
use stellar_scene::freshness::LatestSceneRevision;
use stellar_scene::geometry::{FillRule, PathCommand, PathGeometry};
use stellar_scene::handle::SnapshotHandle;
use stellar_scene::node::{CornerRadii, ImageCrop, ImageFit, ImageSampling, TextLayoutKey};
use stellar_scene::spatial::SceneSpatialIndex;
use stellar_scene::stack::{Opacity, Paint, PaintLayer, SceneBlendMode, StrokeCap};
use stellar_scene::visibility::{visible_candidates, Viewport};
use stellar_scene::{SceneSnapshot, Size2, Vec2};

const VIEW_W: u32 = 1920;
const VIEW_H: u32 = 1080;
const COLS: usize = 50;
const ROWS: usize = 40;
const NODES_PER_CARD: usize = 10;
const HIDDEN_CARDS: usize = 100;
const OFFSCREEN_CARDS: usize = 100;
const SURFACE_FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;
const ALLOW_MISSING_METAL_ENV: &str = "STELLAR_ALLOW_MISSING_METAL";

/// 아키텍처 §6.2 단계 예산(µs, 참고 값): (scene 목표, 상한), (render graph 목표, 상한), (present 목표, 상한).
const BUDGET_US: [(&str, u64, u64); 3] = [
    ("scene", 1000, 1750),
    ("render_graph", 750, 1250),
    ("present", 200, 500),
];

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y).expect("finite")
}

fn size(w: f32, h: f32) -> Size2 {
    Size2::new(w, h).expect("size")
}

fn translate(x: f32, y: f32) -> Transform2D {
    Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).expect("transform")
}

fn color(r: f32) -> Color {
    Color::new(ColorSpace::Srgb, r, 0.5, 0.25, 1.0).expect("color")
}

fn ellipse(w: f32, h: f32) -> PathGeometry {
    let k = 0.552_284_8f32;
    let (rx, ry) = (w / 2.0, h / 2.0);
    PathGeometry::new(vec![
        PathCommand::MoveTo(v(w, ry)),
        PathCommand::CubicTo {
            c1: v(w, ry + k * ry),
            c2: v(rx + k * rx, h),
            to: v(rx, h),
        },
        PathCommand::CubicTo {
            c1: v(rx - k * rx, h),
            c2: v(0.0, ry + k * ry),
            to: v(0.0, ry),
        },
        PathCommand::CubicTo {
            c1: v(0.0, ry - k * ry),
            c2: v(rx - k * rx, 0.0),
            to: v(rx, 0.0),
        },
        PathCommand::CubicTo {
            c1: v(rx + k * rx, 0.0),
            c2: v(w, ry - k * ry),
            to: v(w, ry),
        },
        PathCommand::Close,
    ])
    .expect("ellipse")
}

fn polygon(points: &[(f32, f32)], close: bool) -> PathGeometry {
    let mut commands = vec![PathCommand::MoveTo(v(points[0].0, points[0].1))];
    commands.extend(
        points[1..]
            .iter()
            .map(|&(x, y)| PathCommand::LineTo(v(x, y))),
    );
    if close {
        commands.push(PathCommand::Close);
    }
    PathGeometry::new(commands).expect("polygon")
}

fn paint(fill: bool, stroke: f32) -> ModifierPaint {
    ModifierPaint {
        fills: if fill {
            vec![Paint::Solid(color(0.75))]
        } else {
            Vec::new()
        },
        strokes: if stroke > 0.0 {
            vec![StrokeInput {
                paint: Paint::Solid(color(0.1)),
                width: stroke,
            }]
        } else {
            Vec::new()
        },
        shadows: Vec::new(),
        opacity: Opacity::OPAQUE,
        blend: SceneBlendMode::Normal,
        layer_blur: BlurRadius::ZERO,
    }
}

fn node_input(
    source: &mut u64,
    x: f32,
    y: f32,
    visible: bool,
    fill: bool,
    stroke: f32,
) -> NodeInput {
    *source += 1;
    let mut bytes = [0u8; 16];
    bytes[8..].copy_from_slice(&source.to_be_bytes());
    let mut node = NodeInput::new(StableId::from_bytes(bytes), translate(x, y));
    node.visible = visible;
    node.paint = paint(fill, stroke);
    node
}

/// (x, y) 원점의 36×25 card: Frame + 자식 9개(XI.3.6 benchmark와 같은 구성).
fn card(
    builder: &mut SceneBuilder,
    root: &mut SceneScope,
    source: &mut u64,
    x: f32,
    y: f32,
    visible: bool,
) {
    let frame = FrameInput {
        node: node_input(source, x, y, visible, false, 0.0),
        size: size(36.0, 25.0),
        corner_radii: CornerRadii::new(4.0, 4.0, 4.0, 4.0).expect("radii"),
        background: Some(color(0.9)),
        border: None,
        clip: FrameClipInput::Bounds,
    };
    let mut scope = builder.begin_frame(root, &frame).expect("frame");
    let shape = |source: &mut u64, x, y, geometry: PathGeometry, fill, stroke| ShapeInput {
        node: node_input(source, x, y, true, fill, stroke),
        layout_rect: Rect::new(Vec2::ZERO, size(10.0, 10.0)),
        geometry,
        cap: StrokeCap::Butt,
    };
    let parent = scope.scope_mut();
    let shapes = [
        shape(source, 0.0, 0.0, ellipse(12.0, 12.0), true, 0.0),
        shape(
            source,
            13.0,
            0.0,
            polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true),
            true,
            1.0,
        ),
        shape(source, 28.0, 13.0, ellipse(7.0, 7.0), false, 1.5),
        shape(
            source,
            30.0,
            20.0,
            polygon(&[(0.0, 0.0), (5.0, 0.0), (5.0, 4.0), (0.0, 4.0)], true),
            true,
            0.0,
        ),
    ];
    for input in &shapes {
        builder.build_shape(parent, input).expect("shape");
    }
    let paths = [
        PathInput {
            node: node_input(source, 24.0, 0.0, true, true, 0.0),
            layout_rect: Rect::new(Vec2::ZERO, size(11.0, 10.0)),
            geometry: polygon(&[(0.0, 10.0), (5.5, 0.0), (11.0, 10.0)], true),
            fill_rule: FillRule::EvenOdd,
        },
        PathInput {
            node: node_input(source, 1.0, 12.0, true, false, 1.0),
            layout_rect: Rect::new(Vec2::ZERO, size(34.0, 0.0)),
            geometry: polygon(&[(0.0, 0.0), (17.0, 0.0), (34.0, 0.5)], false),
            fill_rule: FillRule::NonZero,
        },
    ];
    for input in &paths {
        builder.build_path(parent, input).expect("path");
    }
    for (y, w) in [(13.0, 18.0), (19.0, 12.0)] {
        let input = TextInput {
            node: node_input(source, 0.0, y, true, true, 0.0),
            layout: TextLayoutKey::from_bytes([y as u8; 16]),
            size: size(w, 5.0),
        };
        builder.build_text(parent, &input).expect("text");
    }
    let image = ImageInput {
        node: node_input(source, 19.0, 13.0, true, false, 0.0),
        asset: AssetRef::new(StableId::from_bytes([7; 16])),
        size: size(8.0, 11.0),
        fit: ImageFit::Cover,
        crop: ImageCrop::FULL,
        sampling: ImageSampling::Linear,
        color_space: None,
    };
    builder.build_image(parent, &image).expect("image");
    builder.finish_frame(root, scope).expect("finish frame");
}

/// 2만 가시 node scene 전체를 만듭니다(`revision`은 scene revision).
fn build_scene(revision: u64) -> (SceneSnapshot, u64) {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    let mut source = 0u64;
    let (cell_w, cell_h) = (VIEW_W as f32 / COLS as f32, VIEW_H as f32 / ROWS as f32);
    for row in 0..ROWS {
        for col in 0..COLS {
            card(
                &mut builder,
                &mut root,
                &mut source,
                col as f32 * cell_w + 1.0,
                row as f32 * cell_h + 1.0,
                true,
            );
        }
    }
    for i in 0..HIDDEN_CARDS {
        let (x, y) = (
            (i % COLS) as f32 * cell_w + 1.0,
            (i / COLS) as f32 * cell_h + 1.0,
        );
        card(&mut builder, &mut root, &mut source, x, y, false);
    }
    for i in 0..OFFSCREEN_CARDS {
        card(
            &mut builder,
            &mut root,
            &mut source,
            3_000.0 + i as f32 * 40.0,
            100.0,
            true,
        );
    }
    let total = builder.built_count();
    let snapshot = SceneSnapshot::new(
        DocumentRevision(revision),
        SceneRevision(revision),
        builder.finish(root),
    )
    .expect("snapshot");
    (snapshot, total)
}

/// batch → draw command 대리 encoder(모듈 문서). buffer는 frame 사이에 재사용합니다.
struct StandInEncoder {
    plan: OutputColorPlan,
    solid: Option<(BufferHandle, u64)>,
    textured: Option<(BufferHandle, u64)>,
    uniforms: Option<(BufferHandle, u64)>,
    dummy: TextureHandle,
    solid_bytes: Vec<u8>,
    textured_bytes: Vec<u8>,
    uniform_bytes: Vec<u8>,
}

/// 단계별 draw 통계.
#[derive(Debug, Clone, Copy, Default)]
struct EncodeStats {
    draws: usize,
    commands: usize,
}

fn ensure(
    backend: &mut CheckedBackend<MetalBackend>,
    slot: &mut Option<(BufferHandle, u64)>,
    usage: BufferUsage,
    data: &[u8],
) -> Result<Option<BufferHandle>, BackendError> {
    if data.is_empty() {
        return Ok(slot.map(|(handle, _)| handle));
    }
    let needed = data.len() as u64;
    let handle = match *slot {
        Some((handle, capacity)) if capacity >= needed => handle,
        previous => {
            if let Some((old, _)) = previous {
                backend.destroy_buffer(old)?;
            }
            let capacity = needed.next_power_of_two();
            let handle = backend.create_buffer(&BufferDesc {
                size: capacity,
                usage,
            })?;
            *slot = Some((handle, capacity));
            handle
        }
    };
    backend.update_buffer(handle, 0, data)?;
    Ok(Some(handle))
}

fn is_solid_program(program: ShaderProgram) -> bool {
    matches!(
        program,
        ShaderProgram::SolidFill | ShaderProgram::ClipStencil | ShaderProgram::ClipMask
    )
}

impl StandInEncoder {
    fn new(backend: &mut CheckedBackend<MetalBackend>, plan: OutputColorPlan) -> StandInEncoder {
        let dummy = backend
            .create_texture(&GpuTextureDesc {
                width: 1,
                height: 1,
                format: TextureFormat::Rgba8UnormSrgb,
                mip_levels: 1,
                samples: SampleCount::X1,
                usage: TextureUsage {
                    sampled: true,
                    render_target: false,
                },
            })
            .expect("dummy texture");
        backend
            .update_texture(
                dummy,
                TextureRegion {
                    mip_level: 0,
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                &[200, 200, 200, 255],
            )
            .expect("dummy upload");
        StandInEncoder {
            plan,
            solid: None,
            textured: None,
            uniforms: None,
            dummy,
            solid_bytes: Vec::new(),
            textured_bytes: Vec::new(),
            uniform_bytes: Vec::new(),
        }
    }

    /// frame plan·batch를 command로 바꾸고 backend에 encode합니다.
    fn encode(
        &mut self,
        backend: &mut CheckedBackend<MetalBackend>,
        frame_plan: &FramePlan,
        batches: &[PassBatches],
        frame: FrameId,
    ) -> Result<(EncodeOutcome, EncodeStats), BackendError> {
        self.solid_bytes.clear();
        self.textured_bytes.clear();
        self.uniform_bytes.clear();
        // (pass, [(command 또는 draw 자리)]) — buffer handle이 정해진 뒤 채웁니다.
        enum Pending {
            Command(DrawCommand),
            Draw {
                solid: bool,
                first: u32,
                uniform: u64,
            },
        }
        let mut pending: Vec<(stellar_renderer_core::render_graph::PassId, Vec<Pending>)> =
            Vec::new();
        let mut stats = EncodeStats::default();
        for pass_batches in batches {
            let Some(pass) = frame_plan
                .passes
                .iter()
                .find(|p| p.pass == pass_batches.pass)
            else {
                continue;
            };
            let total = pass.draws.len().max(1) as f32;
            let mut list = Vec::new();
            for batch in &pass_batches.batches {
                let key = batch.key;
                list.push(Pending::Command(DrawCommand::SetPipeline(key.pipeline)));
                list.push(Pending::Command(DrawCommand::SetScissor(match key.clip {
                    ClipKey::None => None,
                    ClipKey::Scissor(rect) | ClipKey::Mask { scissor: rect } => Some(rect),
                    ClipKey::Stencil { scissor, .. } => Some(scissor),
                })));
                if let ClipKey::Stencil { reference, .. } = key.clip {
                    list.push(Pending::Command(DrawCommand::SetStencilReference(
                        reference,
                    )));
                }
                match key.texture {
                    TextureKey::None => {}
                    TextureKey::Resource(_) | TextureKey::GlyphAtlas => {
                        list.push(Pending::Command(DrawCommand::BindTexture {
                            slot: 0,
                            texture: TextureSource::Handle(self.dummy),
                            sampler: PLACEHOLDER_SAMPLER,
                        }))
                    }
                    TextureKey::Layer(layer) => {
                        let output = frame_plan.layer(layer).expect("layer").output;
                        list.push(Pending::Command(DrawCommand::BindTexture {
                            slot: 0,
                            texture: TextureSource::Graph(output),
                            sampler: PLACEHOLDER_SAMPLER,
                        }))
                    }
                }
                let solid = is_solid_program(key.pipeline.program);
                for draw in &pass.draws[batch.draws.clone()] {
                    let node = draw.source.node();
                    let Some(bounds) = node.world_hit_bounds() else {
                        continue;
                    };
                    let (x0, y0, x1, y1) = (
                        bounds.min_x(),
                        bounds.min_y(),
                        bounds.max_x(),
                        bounds.max_y(),
                    );
                    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y0], [x1, y1], [x0, y1]];
                    let first = if solid {
                        let fill = node.paint().get().layers.first().map(|layer| match layer {
                            PaintLayer::Fill(paint) => *paint,
                            PaintLayer::Stroke(stroke) => stroke.paint(),
                        });
                        let rgba = fill
                            .map(|paint| {
                                to_working(&self.plan, OutputColor::from_paint(paint)).rgba
                            })
                            .unwrap_or([0.0; 4]);
                        let first = (self.solid_bytes.len() / 24) as u32;
                        for corner in corners {
                            push_solid_vertex(&mut self.solid_bytes, corner, rgba);
                        }
                        first
                    } else {
                        let first = (self.textured_bytes.len() / 16) as u32;
                        let uvs = [
                            [0.0, 0.0],
                            [1.0, 0.0],
                            [1.0, 1.0],
                            [0.0, 0.0],
                            [1.0, 1.0],
                            [0.0, 1.0],
                        ];
                        for (corner, uv) in corners.into_iter().zip(uvs) {
                            push_textured_vertex(&mut self.textured_bytes, corner, uv);
                        }
                        first
                    };
                    while !(self.uniform_bytes.len() as u64).is_multiple_of(UNIFORM_ALIGNMENT) {
                        self.uniform_bytes.push(0);
                    }
                    let uniform = self.uniform_bytes.len() as u64;
                    let z = 1.0 - draw.depth as f32 / (total + 1.0);
                    let opacity = draw.opacity.get();
                    self.uniform_bytes
                        .extend(draw_uniforms([opacity; 4], z, MODE_PLAIN, [0.0; 2]));
                    list.push(Pending::Draw {
                        solid,
                        first,
                        uniform,
                    });
                    stats.draws += 1;
                }
            }
            pending.push((pass_batches.pass, list));
        }
        let solid = ensure(
            backend,
            &mut self.solid,
            BufferUsage::Vertex,
            &self.solid_bytes,
        )?;
        let textured = ensure(
            backend,
            &mut self.textured,
            BufferUsage::Vertex,
            &self.textured_bytes,
        )?;
        let uniforms = ensure(
            backend,
            &mut self.uniforms,
            BufferUsage::Uniform,
            &self.uniform_bytes,
        )?;
        let mut passes = Vec::with_capacity(pending.len());
        for (pass, list) in pending {
            let mut commands = Vec::with_capacity(list.len() * 3);
            let mut bound: Option<bool> = None;
            for item in list {
                match item {
                    Pending::Command(command) => commands.push(command),
                    Pending::Draw {
                        solid: is_solid,
                        first,
                        uniform,
                    } => {
                        let buffer =
                            if is_solid { solid } else { textured }.expect("vertex buffer");
                        if bound != Some(is_solid) {
                            commands.push(DrawCommand::SetVertexBuffer { buffer, offset: 0 });
                            bound = Some(is_solid);
                        }
                        commands.push(DrawCommand::SetUniforms {
                            buffer: uniforms.expect("uniform buffer"),
                            offset: uniform,
                            size: DRAW_UNIFORMS_SIZE,
                        });
                        commands.push(DrawCommand::Draw {
                            vertex_start: first,
                            vertex_count: 6,
                            instance_count: 1,
                        });
                    }
                }
            }
            stats.commands += commands.len();
            passes.push(PassCommands { pass, commands });
        }
        let output = frame_plan.graph.output().expect("graph output");
        let imports = [ImportBinding {
            resource: output.resource,
            target: ImportTarget::Surface,
        }];
        let outcome = backend.encode(&FrameEncodeRequest {
            frame,
            graph: &frame_plan.graph,
            passes: &passes,
            imports: &imports,
        })?;
        Ok((outcome, stats))
    }
}

/// nearest-rank 백분위수.
fn percentile(sorted: &[u128], q: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

fn stats_json(values: &[u128]) -> String {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let total: u128 = sorted.iter().sum();
    let mean = if sorted.is_empty() {
        0
    } else {
        total / sorted.len() as u128
    };
    format!(
        "{{\"p50\":{},\"p95\":{},\"p99\":{},\"max\":{},\"mean\":{}}}",
        percentile(&sorted, 0.50),
        percentile(&sorted, 0.95),
        percentile(&sorted, 0.99),
        sorted.last().copied().unwrap_or(0),
        mean
    )
}

/// 단계 이름(JSON key 순서).
const STAGES: [&str; 6] = [
    "scene_build",
    "visibility",
    "render_graph",
    "encode",
    "submit",
    "total",
];

struct Report {
    json: String,
    frames: usize,
    visible: usize,
    total_nodes: u64,
    draws: usize,
    max_queue_depth: usize,
    stage_samples: [usize; 6],
}

fn backend_or_skip() -> Option<CheckedBackend<MetalBackend>> {
    let config = DeviceConfig {
        surface: SurfaceConfig {
            size: SurfaceSize {
                width: VIEW_W,
                height: VIEW_H,
            },
            format: SURFACE_FORMAT,
            color_space: ColorSpace::Srgb,
        },
        frames_in_flight: 3,
        timestamps: false,
    };
    let result = MetalBackend::with_detached_layer(LayerOptions::DEFAULT).and_then(|backend| {
        let mut backend = CheckedBackend::new(backend);
        backend.init(&config)?;
        Ok(backend)
    });
    match result {
        Ok(backend) => Some(backend),
        Err(error) if std::env::var_os(ALLOW_MISSING_METAL_ENV).is_some_and(|v| v == "1") => {
            eprintln!("Metal 초기화 실패({error}): {ALLOW_MISSING_METAL_ENV}=1이라 benchmark를 검증 없이 건너뜁니다");
            None
        }
        Err(error) => panic!(
            "Metal 초기화 실패({error}). GPU 없는 환경에서 건너뛰려면 {ALLOW_MISSING_METAL_ENV}=1을 설정하십시오"
        ),
    }
}

fn run(backend: &mut CheckedBackend<MetalBackend>, frames: usize, warmup: usize) -> Report {
    let caps = backend.inner().capabilities().expect("capabilities");
    let plan = OutputColorPlan::negotiate(ColorSpace::Srgb, &caps);
    let mut encoder = StandInEncoder::new(backend, plan);
    let latest = LatestSceneRevision::new();
    let mut scheduler = FrameScheduler::new(latest.clone());
    let mut recorder = PresentRecorder::new();
    let viewport = Viewport::new(size(VIEW_W as f32, VIEW_H as f32), Transform2D::identity())
        .expect("viewport");
    let clip_caps = ClipCapabilities::from_device(&caps, SampleCount::X1);
    let target = TargetInfo {
        format: SURFACE_FORMAT,
        samples: SampleCount::X1,
    };
    let mut samples: [Vec<u128>; 6] = Default::default();
    let (
        mut visible,
        mut total_nodes,
        mut draws,
        mut batches_total,
        mut passes_total,
        mut commands,
    ) = (0, 0, 0, 0, 0, 0);
    for frame in 0..warmup + frames {
        let revision = frame as u64 + 1;
        let start = Instant::now();
        let (snapshot, nodes) = build_scene(revision);
        let built = Instant::now();
        let index = SceneSpatialIndex::build(&snapshot);
        let draw_list = visible_candidates(&index, &viewport);
        let culled = Instant::now();
        let frame_plan = compile_frame(&CompileInput {
            snapshot: &snapshot,
            viewport: &viewport,
            visible: Some(&draw_list),
            surface_format: SURFACE_FORMAT,
            clear: ClearColor::TRANSPARENT,
            boundaries: EffectBoundaries::Minimal,
            clip_caps,
        })
        .expect("compile frame");
        let batches = batch_frame(&frame_plan, target);
        let compiled = Instant::now();
        latest.publish(SceneRevision(revision));
        scheduler.offer(SnapshotHandle::new(snapshot));
        let mut encode_ns = 0u128;
        let mut encode_stats = EncodeStats::default();
        let tick_start = Instant::now();
        let outcome = scheduler
            .tick(backend, |backend, _snapshot, frame_id| {
                let encode_start = Instant::now();
                let (outcome, stats) = encoder.encode(backend, &frame_plan, &batches, frame_id)?;
                encode_ns = encode_start.elapsed().as_nanos();
                encode_stats = stats;
                Ok(outcome)
            })
            .expect("tick");
        let ticked = Instant::now();
        recorder.record(&scheduler);
        assert!(
            matches!(outcome, TickOutcome::Submitted { .. }),
            "frame {frame}: {outcome:?}"
        );
        // GPU 완료(측정 밖): present queue를 비워 다음 frame이 throttle되지 않게 합니다.
        let progress = backend.inner_mut().wait_idle().expect("wait");
        scheduler.observe(progress);
        if frame < warmup {
            continue;
        }
        let tick_ns = (ticked - tick_start).as_nanos();
        let values = [
            (built - start).as_nanos(),
            (culled - built).as_nanos(),
            (compiled - culled).as_nanos(),
            encode_ns,
            tick_ns.saturating_sub(encode_ns),
            (built - start).as_nanos()
                + (culled - built).as_nanos()
                + (compiled - culled).as_nanos()
                + tick_ns,
        ];
        for (sample, value) in samples.iter_mut().zip(values) {
            sample.push(value);
        }
        visible = draw_list.items.len();
        total_nodes = nodes;
        draws = encode_stats.draws;
        commands = encode_stats.commands;
        batches_total = batch_count(&batches);
        passes_total = frame_plan.passes.len();
    }
    let metrics = recorder.finish(&scheduler);
    metrics
        .assert_queue_depth()
        .expect("present queue 깊이 ≤ 1");
    metrics.check_accounting().expect("present 지표 집계");
    let mut json = String::new();
    let _ = write!(
        json,
        "{{\"benchmark\":\"render_stages\",\"wbs\":\"XI.11.4\",\"profile\":\"{}\",\"backend\":\"metal\",\"frames\":{},\"warmup\":{},",
        if cfg!(debug_assertions) { "debug" } else { "release" },
        frames,
        warmup
    );
    let _ = write!(
        json,
        "\"scene\":{{\"nodes\":{},\"visible_nodes\":{},\"hidden_cards\":{},\"offscreen_cards\":{},\"viewport\":[{},{}],\"surface_format\":\"bgra8_unorm_srgb\"}},",
        total_nodes, visible, HIDDEN_CARDS, OFFSCREEN_CARDS, VIEW_W, VIEW_H
    );
    let _ = write!(
        json,
        "\"work\":{{\"passes\":{passes_total},\"batches\":{batches_total},\"draws\":{draws},\"commands\":{commands}}},\"stages_ns\":{{"
    );
    for (i, (name, values)) in STAGES.iter().zip(samples.iter()).enumerate() {
        let _ = write!(
            json,
            "{}\"{}\":{}",
            if i == 0 { "" } else { "," },
            name,
            stats_json(values)
        );
    }
    json.push_str("},\"budget_us\":{");
    for (i, (name, target, limit)) in BUDGET_US.iter().enumerate() {
        let _ = write!(
            json,
            "{}\"{name}\":{{\"target\":{target},\"limit\":{limit}}}",
            if i == 0 { "" } else { "," }
        );
    }
    json.push_str("},");
    let present = metrics.to_json();
    // `{"present_queue":{…}}`의 바깥 중괄호를 벗겨 넣습니다.
    json.push_str(&present[1..present.len() - 1]);
    json.push('}');
    Report {
        json,
        frames,
        visible,
        total_nodes,
        draws,
        max_queue_depth: metrics.max_depth,
        stage_samples: std::array::from_fn(|i| samples[i].len()),
    }
}

fn emit(report: &Report) {
    println!("{}", report.json);
    if let Ok(path) = std::env::var("STELLAR_RENDER_BENCH_OUT") {
        std::fs::write(&path, format!("{}\n", report.json)).expect("benchmark JSON 쓰기");
    }
}

#[test]
fn stage_json_reports_percentiles_for_each_stage_on_20k_visible_scene() {
    // 검증 기준(XI.11.4): 단계별 p50/p95/p99 JSON 생성(frame 수를 줄인 구성 확인, 시간 판정 없음).
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let report = run(&mut backend, 3, 1);
    assert_eq!(report.frames, 3);
    assert_eq!(
        report.total_nodes,
        ((COLS * ROWS + HIDDEN_CARDS + OFFSCREEN_CARDS) * NODES_PER_CARD) as u64
    );
    assert_eq!(
        report.visible,
        COLS * ROWS * NODES_PER_CARD,
        "가시 node 2만"
    );
    assert!(
        report.draws >= report.visible,
        "가시 node마다 draw가 하나 이상"
    );
    assert!(report.max_queue_depth <= 1);
    assert_eq!(report.stage_samples, [3; 6]);
    for stage in STAGES {
        let key = format!("\"{stage}\":{{\"p50\":");
        assert!(report.json.contains(&key), "{stage}: {}", report.json);
    }
    for key in [
        "\"p95\":",
        "\"p99\":",
        "\"present_queue\":{\"limit\":1",
        "\"budget_us\":",
        "\"visible_nodes\":20000",
    ] {
        assert!(report.json.contains(key), "{key}: {}", report.json);
    }
    assert!(report.json.starts_with('{') && report.json.ends_with('}'));
    let depth = report.json.chars().fold(0i32, |d, c| match c {
        '{' => d + 1,
        '}' => d - 1,
        _ => d,
    });
    assert_eq!(depth, 0, "중괄호 짝: {}", report.json);
    emit(&report);
}

#[test]
#[ignore = "release 측정 전용: cargo test --release -p stellar-bench-fixtures --test render_stage_benchmark -- --ignored --nocapture"]
fn render_stages_on_20k_visible_nodes() {
    let Some(mut backend) = backend_or_skip() else {
        return;
    };
    let report = run(&mut backend, 120, 10);
    emit(&report);
}

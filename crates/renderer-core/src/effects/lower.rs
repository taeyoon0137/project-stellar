//! fixture → render graph + draw command lowering (XI.11.1, ADR-0102 결정 6).
//!
//! [`lower`]는 [`RendererBackend`] 하나에 fixture가 쓰는 buffer·texture를 만들고 올린 뒤, 한 frame의
//! [`CompiledRenderGraph`]와 pass별 [`DrawCommand`] 목록을 돌려줍니다. backend 중립이라 Metal harness와 후속 D3D12
//! harness가 같은 lowering을 씁니다. 그리기 의미는 CPU 기준([`super::raster`])과 같습니다.
//!
//! - target: fixture format의 external texture 하나(`ImportKind::External`, 고정 크기). main pass가 clear 색으로 지우고
//!   painter 순서로 그립니다.
//! - 단색 mesh: `SolidFill` + premultiplied over. [`Coverage::Union`] mesh는 stencil union입니다. `IncrementWhereEqual`
//!   (reference 0)로 그려 pixel마다 처음 한 번만 칠하고, 같은 mesh를 `ClipStencil` + `DecrementWhereEqual`로 다시 그려
//!   stencil을 0으로 되돌립니다. stencil이 필요한 pass에만 depth·stencil transient를 붙입니다.
//! - 이미지: `Rgba8UnormSrgb` texture([`ImageData::texture_bytes`]) + `Texture` program, tint = 불투명도.
//! - glyph: `R8Unorm` coverage texture + `GlyphMask` program(Nearest), tint = 글자 색 working 값.
//! - layer: 내용을 자기 pass의 `Rgba16Float` transient(`pass_compiler::LAYER_FORMAT`, 투명 clear)에 그리고, 부모 pass가 그
//!   version을 읽어 target 전체 quad로 합성합니다. program·blend state는 [`crate::blend::composite_program`]입니다.
//!   layer pass는 부모보다 먼저 끝나므로 기록 목록에서 앞에 옵니다(graph가 resource 흐름으로 순서를 정함).
//! - 정점·uniform byte 배치는 [`super::abi`]입니다.

use std::fmt;

use crate::backend::{
    BackendError, BlendState, BufferDesc, BufferHandle, BufferUsage, DepthMode, DrawCommand,
    FrameEncodeRequest, FrameId, GpuTextureDesc, ImportBinding, ImportTarget, PassCommands,
    PipelineKey, RendererBackend, ShaderProgram, StencilMode, TextureHandle, TextureRegion,
    TextureSource, TextureUsage,
};
use crate::color::to_working;
use crate::output_color::OutputColorPlan;
use crate::placeholder::{SamplerDesc, TextureFilter, TextureWrap};
use crate::render_graph::{
    AttachmentSize, CompiledRenderGraph, GraphBuildError, ImportKind, LoadOp, PassId, PassKind,
    RenderGraphBuilder, RenderGraphError, ResourceVersion, SampleCount, StoreOp, TextureDesc,
    TextureFormat,
};

use super::abi::{
    draw_uniforms, push_solid_vertex, push_textured_vertex, DEFAULT_DEPTH, DRAW_UNIFORMS_SIZE,
    MODE_PLAIN, UNIFORM_ALIGNMENT,
};
use super::fixture::{
    Coverage, FillOp, Fixture, FixtureError, GlyphOp, ImageOp, LayerOp, Op, Paint,
    IMAGE_TEXTURE_FORMAT,
};
use crate::blend::{composite_blend_state, composite_program};
use crate::pass_compiler::LAYER_FORMAT;
use stellar_scene::stack::SceneBlendMode;

/// lowering 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum LowerError {
    /// fixture 계약 위반.
    Fixture(FixtureError),
    /// backend 호출 실패.
    Backend(BackendError),
    /// graph 구성 실패.
    Graph(&'static str),
}

impl fmt::Display for LowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LowerError::Fixture(error) => write!(f, "fixture 오류: {error:?}"),
            LowerError::Backend(error) => write!(f, "backend 오류: {error}"),
            LowerError::Graph(detail) => write!(f, "graph 오류: {detail}"),
        }
    }
}

impl From<BackendError> for LowerError {
    fn from(error: BackendError) -> LowerError {
        LowerError::Backend(error)
    }
}

impl From<FixtureError> for LowerError {
    fn from(error: FixtureError) -> LowerError {
        LowerError::Fixture(error)
    }
}

impl From<GraphBuildError> for LowerError {
    fn from(_: GraphBuildError) -> LowerError {
        LowerError::Graph("graph builder 호출이 잘못되었습니다")
    }
}

impl From<RenderGraphError> for LowerError {
    fn from(_: RenderGraphError) -> LowerError {
        LowerError::Graph("graph compile 실패")
    }
}

/// lowering 결과: 한 frame에 필요한 모든 것.
#[derive(Debug)]
pub struct LoweredFrame {
    /// compile한 graph.
    pub graph: CompiledRenderGraph,
    /// pass별 command.
    pub passes: Vec<PassCommands>,
    /// output을 target texture에 묶는 binding.
    pub imports: Vec<ImportBinding>,
    /// fixture target texture(readback 대상).
    pub target: TextureHandle,
    buffers: Vec<BufferHandle>,
    textures: Vec<TextureHandle>,
}

impl LoweredFrame {
    /// encode 요청.
    pub fn request(&self, frame: FrameId) -> FrameEncodeRequest<'_> {
        FrameEncodeRequest {
            frame,
            graph: &self.graph,
            passes: &self.passes,
            imports: &self.imports,
        }
    }

    /// 만든 buffer·texture(target 포함)를 모두 파괴합니다.
    pub fn release<B: RendererBackend + ?Sized>(self, backend: &mut B) -> Result<(), BackendError> {
        for buffer in self.buffers {
            backend.destroy_buffer(buffer)?;
        }
        for texture in self.textures {
            backend.destroy_texture(texture)?;
        }
        backend.destroy_texture(self.target)
    }

    /// draw command 수(진단용).
    pub fn draw_count(&self) -> usize {
        self.passes
            .iter()
            .flat_map(|pass| pass.commands.iter())
            .filter(|command| matches!(command, DrawCommand::Draw { .. }))
            .count()
    }
}

/// vertex stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stream {
    Solid,
    Textured,
}

/// texture 참조(upload 순번 또는 graph version).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TexRef {
    Upload(usize),
    Graph(ResourceVersion),
}

/// backend handle이 정해지기 전 command.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Step {
    Pipeline(PipelineKey),
    StencilReference(u8),
    Texture {
        slot: u8,
        texture: TexRef,
        sampler: SamplerDesc,
    },
    Draw {
        stream: Stream,
        first: u32,
        count: u32,
        uniform_offset: u64,
        uniform_size: u32,
    },
}

/// texture upload.
#[derive(Debug, Clone)]
pub(crate) struct Upload {
    pub(crate) desc: GpuTextureDesc,
    pub(crate) bytes: Vec<u8>,
}

/// 기록 중인 pass 하나.
#[derive(Debug)]
pub(crate) struct PassRecord {
    pub(crate) id: PassId,
    pub(crate) format: TextureFormat,
    pub(crate) stencil: bool,
    pub(crate) steps: Vec<Step>,
}

/// lowering 상태.
pub(crate) struct Recorder {
    pub(crate) plan: OutputColorPlan,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) solid: Vec<u8>,
    pub(crate) textured: Vec<u8>,
    pub(crate) uniforms: Vec<u8>,
    pub(crate) uploads: Vec<Upload>,
    pub(crate) builder: RenderGraphBuilder,
    /// 끝난 pass 기록(layer pass가 먼저 끝납니다).
    pub(crate) records: Vec<PassRecord>,
}

/// clamp sampler.
pub(crate) const fn clamp_sampler(filter: TextureFilter) -> SamplerDesc {
    SamplerDesc {
        filter,
        wrap: TextureWrap::ClampToEdge,
        mipmaps: false,
    }
}

/// `program`·`blend`·`stencil` pipeline key(depth 미사용, 1x).
pub(crate) const fn key(
    program: ShaderProgram,
    blend: BlendState,
    stencil: StencilMode,
    format: TextureFormat,
) -> PipelineKey {
    PipelineKey {
        program,
        blend,
        depth: DepthMode::Disabled,
        stencil,
        color_format: format,
        samples: SampleCount::X1,
    }
}

impl Recorder {
    pub(crate) fn new(plan: OutputColorPlan, width: u32, height: u32) -> Recorder {
        Recorder {
            plan,
            width,
            height,
            solid: Vec::new(),
            textured: Vec::new(),
            uniforms: Vec::new(),
            uploads: Vec::new(),
            builder: RenderGraphBuilder::new(),
            records: Vec::new(),
        }
    }

    /// uniform block을 덧붙이고 (offset, size)를 돌려줍니다.
    pub(crate) fn uniform(&mut self, bytes: Vec<u8>) -> (u64, u32) {
        while !(self.uniforms.len() as u64).is_multiple_of(UNIFORM_ALIGNMENT) {
            self.uniforms.push(0);
        }
        let offset = self.uniforms.len() as u64;
        let size = bytes.len() as u32;
        self.uniforms.extend(bytes);
        (offset, size)
    }

    /// 기본 tint uniform.
    pub(crate) fn tint(&mut self, tint: [f32; 4]) -> (u64, u32) {
        self.uniform(draw_uniforms(tint, DEFAULT_DEPTH, MODE_PLAIN, [0.0; 2]))
    }

    /// 단색 정점 삼각형 목록을 덧붙이고 (첫 정점, 수)를 돌려줍니다.
    pub(crate) fn solid_vertices(&mut self, positions: &[[f32; 2]], color: [f32; 4]) -> (u32, u32) {
        let first = (self.solid.len() / 24) as u32;
        for p in positions {
            push_solid_vertex(&mut self.solid, *p, color);
        }
        (first, positions.len() as u32)
    }

    /// uv 정점 quad(삼각형 2개)를 덧붙입니다.
    pub(crate) fn textured_quad(&mut self, rect: [f32; 4], uv: [f32; 4]) -> (u32, u32) {
        let first = (self.textured.len() / 16) as u32;
        let [x0, y0, x1, y1] = rect;
        let [u0, v0, u1, v1] = uv;
        for (p, t) in [
            ([x0, y0], [u0, v0]),
            ([x1, y0], [u1, v0]),
            ([x1, y1], [u1, v1]),
            ([x0, y0], [u0, v0]),
            ([x1, y1], [u1, v1]),
            ([x0, y1], [u0, v1]),
        ] {
            push_textured_vertex(&mut self.textured, p, t);
        }
        (first, 6)
    }

    /// sampled texture upload를 예약합니다.
    pub(crate) fn upload(
        &mut self,
        width: u32,
        height: u32,
        format: TextureFormat,
        bytes: Vec<u8>,
    ) -> usize {
        self.uploads.push(Upload {
            desc: GpuTextureDesc {
                width,
                height,
                format,
                mip_levels: 1,
                samples: SampleCount::X1,
                usage: TextureUsage {
                    sampled: true,
                    render_target: false,
                },
            },
            bytes,
        });
        self.uploads.len() - 1
    }

    /// 그리기 목록을 `pass`에 기록합니다.
    pub(crate) fn record_ops(
        &mut self,
        pass: &mut PassRecord,
        ops: &[Op],
    ) -> Result<(), LowerError> {
        for op in ops {
            match op {
                Op::Fill(fill) => self.record_fill(pass, fill),
                Op::Image(image) => self.record_image(pass, image),
                Op::Glyphs(glyph) => self.record_glyph(pass, glyph),
                Op::Layer(layer) => self.record_layer(pass, layer)?,
            }
        }
        Ok(())
    }

    /// layer 내용을 자기 offscreen pass에 그리고 부모 pass에 합성합니다.
    fn record_layer(&mut self, parent: &mut PassRecord, layer: &LayerOp) -> Result<(), LowerError> {
        let texture = self
            .builder
            .transient("conformance layer", self.desc(LAYER_FORMAT));
        let id = self.builder.pass("conformance layer", PassKind::Raster);
        let mut child = PassRecord {
            id,
            format: LAYER_FORMAT,
            stencil: false,
            steps: Vec::new(),
        };
        self.record_ops(&mut child, &layer.content)?;
        let written = self.builder.color(
            id,
            texture,
            LoadOp::Clear,
            crate::render_graph::ClearColor::TRANSPARENT,
            StoreOp::Store,
        )?;
        self.attach_stencil(&child)?;
        self.records.push(child);
        self.builder.read(parent.id, written)?;
        self.composite(parent, written, layer.blend);
        Ok(())
    }

    /// graph texture 전체를 부모 target에 `mode`로 합성합니다(ADR-0102 결정 3).
    fn composite(
        &mut self,
        parent: &mut PassRecord,
        source: ResourceVersion,
        mode: SceneBlendMode,
    ) {
        let (first, count) = self.textured_quad(
            [0.0, 0.0, self.width as f32, self.height as f32],
            [0.0, 0.0, 1.0, 1.0],
        );
        let (uniform_offset, uniform_size) = self.tint([1.0; 4]);
        parent.steps.push(Step::Pipeline(key(
            composite_program(mode),
            composite_blend_state(mode),
            StencilMode::Disabled,
            parent.format,
        )));
        parent.steps.push(Step::Texture {
            slot: 0,
            texture: TexRef::Graph(source),
            sampler: clamp_sampler(TextureFilter::Nearest),
        });
        parent.steps.push(Step::Draw {
            stream: Stream::Textured,
            first,
            count,
            uniform_offset,
            uniform_size,
        });
    }

    /// pass에 stencil이 필요하면 depth·stencil transient를 붙입니다.
    pub(crate) fn attach_stencil(&mut self, pass: &PassRecord) -> Result<(), LowerError> {
        if pass.stencil {
            let depth = self.builder.transient(
                "conformance stencil",
                self.desc(TextureFormat::Depth32FloatStencil8),
            );
            self.builder
                .depth_stencil(pass.id, depth, LoadOp::Clear, StoreOp::Discard)?;
        }
        Ok(())
    }

    fn record_fill(&mut self, pass: &mut PassRecord, fill: &FillOp) {
        if fill.mesh.positions.is_empty() {
            return;
        }
        let color = match &fill.paint {
            Paint::Solid(color) => to_working(&self.plan, *color).rgba,
        };
        let (first, count) = self.solid_vertices(&fill.mesh.positions, color);
        let (uniform_offset, uniform_size) = self.tint([1.0; 4]);
        let draw = Step::Draw {
            stream: Stream::Solid,
            first,
            count,
            uniform_offset,
            uniform_size,
        };
        match fill.coverage {
            Coverage::Disjoint => {
                pass.steps.push(Step::Pipeline(key(
                    ShaderProgram::SolidFill,
                    BlendState::PremultipliedOver,
                    StencilMode::Disabled,
                    pass.format,
                )));
                pass.steps.push(draw);
            }
            Coverage::Union => {
                pass.stencil = true;
                pass.steps.push(Step::Pipeline(key(
                    ShaderProgram::SolidFill,
                    BlendState::PremultipliedOver,
                    StencilMode::IncrementWhereEqual,
                    pass.format,
                )));
                pass.steps.push(Step::StencilReference(0));
                pass.steps.push(draw);
                pass.steps.push(Step::Pipeline(key(
                    ShaderProgram::ClipStencil,
                    BlendState::Replace,
                    StencilMode::DecrementWhereEqual,
                    pass.format,
                )));
                pass.steps.push(draw);
            }
        }
    }

    fn record_image(&mut self, pass: &mut PassRecord, image: &ImageOp) {
        let data = &image.image;
        let upload = self.upload(
            data.width,
            data.height,
            IMAGE_TEXTURE_FORMAT,
            data.texture_bytes(&self.plan),
        );
        let (first, count) = self.textured_quad(image.rect, image.uv);
        let (uniform_offset, uniform_size) = self.tint([image.opacity; 4]);
        pass.steps.push(Step::Pipeline(key(
            ShaderProgram::Texture,
            BlendState::PremultipliedOver,
            StencilMode::Disabled,
            pass.format,
        )));
        pass.steps.push(Step::Texture {
            slot: 0,
            texture: TexRef::Upload(upload),
            sampler: clamp_sampler(image.filter),
        });
        pass.steps.push(Step::Draw {
            stream: Stream::Textured,
            first,
            count,
            uniform_offset,
            uniform_size,
        });
    }

    fn record_glyph(&mut self, pass: &mut PassRecord, glyph: &GlyphOp) {
        let mask = &glyph.mask;
        let upload = self.upload(
            mask.width,
            mask.height,
            TextureFormat::R8Unorm,
            mask.data.clone(),
        );
        let [ox, oy] = glyph.origin.map(|v| v as f32);
        let (first, count) = self.textured_quad(
            [ox, oy, ox + mask.width as f32, oy + mask.height as f32],
            [0.0, 0.0, 1.0, 1.0],
        );
        let color = to_working(&self.plan, glyph.color).rgba;
        let (uniform_offset, uniform_size) = self.tint(color);
        pass.steps.push(Step::Pipeline(key(
            ShaderProgram::GlyphMask,
            BlendState::PremultipliedOver,
            StencilMode::Disabled,
            pass.format,
        )));
        pass.steps.push(Step::Texture {
            slot: 0,
            texture: TexRef::Upload(upload),
            sampler: clamp_sampler(TextureFilter::Nearest),
        });
        pass.steps.push(Step::Draw {
            stream: Stream::Textured,
            first,
            count,
            uniform_offset,
            uniform_size,
        });
    }

    /// 고정 크기 graph texture descriptor.
    pub(crate) fn desc(&self, format: TextureFormat) -> TextureDesc {
        TextureDesc {
            format,
            size: AttachmentSize::Fixed {
                width: self.width,
                height: self.height,
            },
            samples: SampleCount::X1,
        }
    }
}

/// fixture를 `backend`의 한 frame으로 바꿉니다(buffer·texture 생성과 upload 포함).
pub fn lower<B: RendererBackend + ?Sized>(
    backend: &mut B,
    fixture: &Fixture,
) -> Result<LoweredFrame, LowerError> {
    fixture.validate()?;
    let mut recorder = Recorder::new(fixture.plan(), fixture.width, fixture.height);
    let target = recorder.builder.import(
        "conformance target",
        recorder.desc(fixture.format),
        ImportKind::External,
    );
    let main = recorder.builder.pass("conformance main", PassKind::Raster);
    let mut record = PassRecord {
        id: main,
        format: fixture.format,
        stencil: false,
        steps: Vec::new(),
    };
    recorder.record_ops(&mut record, &fixture.ops)?;
    let clear = fixture
        .clear_working()
        .to_clear()
        .ok_or(LowerError::Fixture(FixtureError::Invalid(
            "clear 색이 유한하지 않습니다",
        )))?;
    let written = recorder
        .builder
        .color(main, target, LoadOp::Clear, clear, StoreOp::Store)?;
    recorder.attach_stencil(&record)?;
    recorder.records.push(record);
    recorder.builder.set_output(written);
    let graph = std::mem::take(&mut recorder.builder).finish().compile()?;
    finish(backend, fixture, recorder, graph)
}

/// buffer·texture를 만들고 step을 command로 바꿉니다.
pub(crate) fn finish<B: RendererBackend + ?Sized>(
    backend: &mut B,
    fixture: &Fixture,
    recorder: Recorder,
    graph: CompiledRenderGraph,
) -> Result<LoweredFrame, LowerError> {
    let records = &recorder.records;
    let mut buffers = Vec::new();
    let mut textures = Vec::new();
    let result =
        (|| -> Result<(CompiledRenderGraph, Vec<PassCommands>, TextureHandle), LowerError> {
            let mut make_buffer =
                |usage, data: &[u8]| -> Result<Option<BufferHandle>, BackendError> {
                    if data.is_empty() {
                        return Ok(None);
                    }
                    let handle = backend.create_buffer(&BufferDesc {
                        size: data.len() as u64,
                        usage,
                    })?;
                    buffers.push(handle);
                    Ok(Some(handle))
                };
            let solid = make_buffer(BufferUsage::Vertex, &recorder.solid)?;
            let textured = make_buffer(BufferUsage::Vertex, &recorder.textured)?;
            let uniforms = make_buffer(BufferUsage::Uniform, &recorder.uniforms)?;
            for (handle, data) in [
                (solid, &recorder.solid),
                (textured, &recorder.textured),
                (uniforms, &recorder.uniforms),
            ] {
                if let Some(handle) = handle {
                    backend.update_buffer(handle, 0, data)?;
                }
            }
            let mut uploaded = Vec::with_capacity(recorder.uploads.len());
            for upload in &recorder.uploads {
                let handle = backend.create_texture(&upload.desc)?;
                textures.push(handle);
                backend.update_texture(
                    handle,
                    TextureRegion {
                        mip_level: 0,
                        x: 0,
                        y: 0,
                        width: upload.desc.width,
                        height: upload.desc.height,
                    },
                    &upload.bytes,
                )?;
                uploaded.push(handle);
            }
            let target = backend.create_texture(&GpuTextureDesc {
                width: fixture.width,
                height: fixture.height,
                format: fixture.format,
                mip_levels: 1,
                samples: SampleCount::X1,
                usage: TextureUsage {
                    sampled: true,
                    render_target: true,
                },
            })?;
            let mut passes = Vec::with_capacity(records.len());
            for record in records {
                let mut commands = Vec::with_capacity(record.steps.len() * 2);
                let mut bound: Option<Stream> = None;
                for step in &record.steps {
                    match *step {
                        Step::Pipeline(key) => commands.push(DrawCommand::SetPipeline(key)),
                        Step::StencilReference(reference) => {
                            commands.push(DrawCommand::SetStencilReference(reference))
                        }
                        Step::Texture {
                            slot,
                            texture,
                            sampler,
                        } => commands.push(DrawCommand::BindTexture {
                            slot,
                            texture: match texture {
                                TexRef::Upload(index) => TextureSource::Handle(uploaded[index]),
                                TexRef::Graph(version) => TextureSource::Graph(version),
                            },
                            sampler,
                        }),
                        Step::Draw {
                            stream,
                            first,
                            count,
                            uniform_offset,
                            uniform_size,
                        } => {
                            let buffer = match stream {
                                Stream::Solid => solid,
                                Stream::Textured => textured,
                            }
                            .ok_or(LowerError::Graph("정점 없는 draw"))?;
                            if bound != Some(stream) {
                                commands.push(DrawCommand::SetVertexBuffer { buffer, offset: 0 });
                                bound = Some(stream);
                            }
                            commands.push(DrawCommand::SetUniforms {
                                buffer: uniforms.ok_or(LowerError::Graph("uniform 없는 draw"))?,
                                offset: uniform_offset,
                                size: uniform_size.max(DRAW_UNIFORMS_SIZE),
                            });
                            commands.push(DrawCommand::Draw {
                                vertex_start: first,
                                vertex_count: count,
                                instance_count: 1,
                            });
                        }
                    }
                }
                passes.push(PassCommands {
                    pass: record.id,
                    commands,
                });
            }
            Ok((graph, passes, target))
        })();
    match result {
        Ok((graph, passes, target)) => {
            let output = graph
                .output()
                .ok_or(LowerError::Graph("graph output이 없습니다"))?;
            Ok(LoweredFrame {
                imports: vec![ImportBinding {
                    resource: output.resource,
                    target: ImportTarget::Texture(target),
                }],
                graph,
                passes,
                target,
                buffers,
                textures,
            })
        }
        Err(error) => {
            for buffer in buffers {
                let _ = backend.destroy_buffer(buffer);
            }
            for texture in textures {
                let _ = backend.destroy_texture(texture);
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use crate::backend::tests::device_config;
    use crate::backend::CheckedBackend;
    use crate::effects::fixture::{solid, srgb, Category, Mesh};

    #[test]
    fn lowering_creates_resources_and_valid_frame() {
        let fixture = Fixture::new("lower-smoke", Category::Frame, 16, 16)
            .with(solid(
                Mesh::rect(2.0, 2.0, 10.0, 10.0),
                srgb(1.0, 0.0, 0.0, 1.0),
                Coverage::Disjoint,
            ))
            .with(solid(
                Mesh::rect(4.0, 4.0, 12.0, 12.0),
                srgb(0.0, 0.0, 1.0, 0.5),
                Coverage::Union,
            ));
        let mut backend = CheckedBackend::new(MockBackend::new());
        backend.init(&device_config(16, 16)).unwrap();
        let lowered = lower(&mut backend, &fixture).unwrap();
        assert_eq!(lowered.passes.len(), 1);
        // 단색 1 + union 2(칠하기·stencil 되돌리기).
        assert_eq!(lowered.draw_count(), 3);
        let pass = lowered.graph.pass(PassId(0)).unwrap();
        assert!(
            pass.depth_stencil.is_some(),
            "union mesh는 stencil을 씁니다"
        );
        let frame = FrameId(1);
        match backend.encode(&lowered.request(frame)).unwrap() {
            crate::backend::EncodeOutcome::Encoded(encoded) => {
                backend.submit(encoded).unwrap();
            }
            other => panic!("{other:?}"),
        }
        lowered.release(&mut backend).unwrap();
    }

    #[test]
    fn invalid_fixture_is_rejected_before_backend_calls() {
        let mut fixture = Fixture::new("bad", Category::Frame, 0, 4);
        fixture.format = TextureFormat::Rgba16Float;
        let mut backend = CheckedBackend::new(MockBackend::new());
        backend.init(&device_config(4, 4)).unwrap();
        assert!(matches!(
            lower(&mut backend, &fixture),
            Err(LowerError::Fixture(_))
        ));
    }
}

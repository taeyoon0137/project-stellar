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
//! - gradient: stop ramp를 `RAMP_WIDTH` texel `Rgba16Float` texture로 올리고 정점 uv.x = ramp 좌표(선형 gradient의 `t`는 위치의
//!   affine 함수)로 `Texture` program이 linear filter·clamp로 읽습니다(ADR-0102 결정 4).
//! - 이미지: `Rgba8UnormSrgb` texture([`ImageData::texture_bytes`]) + `Texture` program, tint = 불투명도.
//! - glyph: `R8Unorm` coverage texture + `GlyphMask` program(Nearest), tint = 글자 색 working 값.
//! - layer: 내용을 자기 pass의 `Rgba16Float` transient(`pass_compiler::LAYER_FORMAT`, 투명 clear)에 그리고, 부모 pass가 그
//!   version을 읽어 target 전체 quad로 합성합니다. program·blend state는 [`crate::blend::composite_program`]입니다.
//!   layer pass는 부모보다 먼저 끝나므로 기록 목록에서 앞에 옵니다(graph가 resource 흐름으로 순서를 정함).
//! - layer 효과(ADR-0102 결정 5): shadow마다 `GaussianBlur` 두 pass(가로는 alpha를 퍼뜨리고, 세로는 shadow 색을 곱함)를 만들고
//!   shadow·내용을 쌓는 pass에 소수 offset quad(Linear)로 그립니다. 상자 shadow는 불투명 상자 원본 pass를 blur하고, 쌓는
//!   pass에서 상자를 stencil로 표시해 상자 밖에만 그립니다. σ가 `MAX_SIGMA`를 넘으면 원본을 2배씩 줄이는 pass(`LayerComposite`
//!   Linear 표본 하나 = 2×2 상자 평균)를 거쳐 낮은 해상도에서 blur하고, shadow quad·layer blur 키우기 pass가 Linear로 다시
//!   키웁니다(ADR-0130 결정 1·2·3). layer blur는 `GaussianBlur` 가로·세로 pass입니다. 합성 단계에서 scissor clip은
//!   `SetScissor`, path clip은 `ClipStencil`(`IncrementWhereEqual`) → 합성(`TestEqual` reference 1) → `DecrementWhereEqual`로 되돌림,
//!   mask·mask clip은 `R8Unorm` coverage pass(`ClipMask` + premultiplied over)를 slot 1에 bind하고 `DrawUniforms.mode = 1`, 불투명도는
//!   tint입니다.
//! - 정점·uniform byte 배치는 [`super::abi`]입니다.

use std::fmt;

use crate::backend::{
    BackendError, BlendState, BufferDesc, BufferHandle, BufferUsage, DepthMode, DrawCommand,
    FrameEncodeRequest, FrameId, GpuTextureDesc, ImportBinding, ImportTarget, PassCommands,
    PipelineKey, RendererBackend, ScissorRect, ShaderProgram, StencilMode, TextureHandle,
    TextureRegion, TextureSource, TextureUsage,
};
use crate::color::{encode_pixel, to_working};
use crate::output_color::OutputColorPlan;
use crate::placeholder::{SamplerDesc, TextureFilter, TextureWrap};
use crate::render_graph::{
    AttachmentSize, CompiledRenderGraph, GraphBuildError, ImportKind, LoadOp, PassId, PassKind,
    RenderGraphBuilder, RenderGraphError, ResourceVersion, SampleCount, StoreOp, TextureDesc,
    TextureFormat,
};

use super::abi::{
    blur_uniforms, draw_uniforms, push_solid_vertex, push_textured_vertex, DEFAULT_DEPTH,
    DRAW_UNIFORMS_SIZE, MODE_MASKED, MODE_PLAIN, UNIFORM_ALIGNMENT,
};
use super::fixture::{
    ClipSpec, Coverage, CoverageShape, FillOp, Fixture, FixtureError, GlyphOp, ImageOp, LayerOp,
    Mesh, Op, Paint, ShadowSource, IMAGE_TEXTURE_FORMAT,
};
use super::gradient::{GradientRamp, RAMP_WIDTH};
use super::window::{plan_layer, EffectSpace, Window};
use crate::blend::{composite_blend_state, composite_program};
use crate::pass_compiler::LAYER_FORMAT;
use stellar_scene::stack::SceneBlendMode;

/// window 전체 사각형(texel 좌표).
fn full_rect(window: &Window) -> [f32; 4] {
    [0.0, 0.0, window.width as f32, window.height as f32]
}

/// lowering 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum LowerError {
    /// fixture 계약 위반.
    Fixture(FixtureError),
    /// backend 호출 실패.
    Backend(BackendError),
    /// graph 구성 실패.
    Graph(&'static str),
    /// backend 공통 경로가 아직 없는 그리기.
    Unsupported(&'static str),
}

impl fmt::Display for LowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LowerError::Fixture(error) => write!(f, "fixture 오류: {error:?}"),
            LowerError::Backend(error) => write!(f, "backend 오류: {error}"),
            LowerError::Graph(detail) => write!(f, "graph 오류: {detail}"),
            LowerError::Unsupported(detail) => write!(f, "지원하지 않는 그리기: {detail}"),
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
    Scissor(Option<ScissorRect>),
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
    /// pass가 덮는 device 영역과 해상도(ADR-0136).
    pub(crate) window: Window,
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
    /// ops 공간(효과 window 계획, ADR-0136).
    pub(crate) space: EffectSpace,
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
    pub(crate) fn new(
        plan: OutputColorPlan,
        width: u32,
        height: u32,
        space: EffectSpace,
    ) -> Recorder {
        Recorder {
            plan,
            width,
            height,
            solid: Vec::new(),
            textured: Vec::new(),
            uniforms: Vec::new(),
            uploads: Vec::new(),
            space,
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

    /// 그리기 목록(device 좌표)을 `pass` window texel 좌표로 옮겨 기록합니다.
    pub(crate) fn record_ops(
        &mut self,
        pass: &mut PassRecord,
        ops: &[Op],
    ) -> Result<(), LowerError> {
        for op in ops {
            match op {
                Op::Fill(fill) => self.record_fill(pass, fill)?,
                Op::Image(image) => self.record_image(pass, image),
                Op::Glyphs(glyph) => self.record_glyph(pass, glyph),
                Op::Layer(layer) => self.record_layer(pass, layer)?,
            }
        }
        Ok(())
    }

    /// layer를 효과 순서(ADR-0102 결정 5)대로 offscreen pass에 그리고 부모 pass에 합성합니다. pass마다 window는
    /// [`plan_layer`]입니다(ADR-0136).
    ///
    /// 내용 pass → (shadow마다 원본 → 줄이기 → alpha blur 가로·색 blur 세로 pass) → shadow·내용 쌓기 pass → (layer blur
    /// 줄이기·가로·세로 pass → 부모 window로 되돌리기) → (mask coverage pass) → 부모에 clip·mask·opacity·blend 합성.
    fn record_layer(&mut self, parent: &mut PassRecord, layer: &LayerOp) -> Result<(), LowerError> {
        let parent_window = parent.window;
        let Some(windows) = plan_layer(layer, &parent_window, &self.space)
            .map_err(|error| LowerError::Fixture(FixtureError::Window(error)))?
        else {
            return Ok(());
        };
        let scissor = match &layer.clip {
            Some(ClipSpec::Scissor {
                x,
                y,
                width,
                height,
            }) => match parent_window.map_scissor(*x, *y, *width, *height) {
                Some(rect) => Some(rect),
                // 부모 window 안에 보이는 곳이 없습니다(CPU 기준도 coverage 0).
                None => return Ok(()),
            },
            _ => None,
        };
        let g = windows.group;
        let content = self.layer_pass("conformance layer", LAYER_FORMAT, g, |rec, pass| {
            rec.record_ops(pass, &layer.content)
        })?;
        let mut group = content;
        if !layer.shadows.is_empty() {
            let mut colored = Vec::with_capacity(layer.shadows.len());
            // 같은 (원본, window)의 원본 pass는 하나만 그립니다.
            let mut boxes: Vec<(&Mesh, Window, ResourceVersion)> = Vec::new();
            let mut contents: Vec<(Window, ResourceVersion)> = Vec::new();
            for (shadow, plan) in layer.shadows.iter().zip(&windows.shadows) {
                let Some(plan) = plan else {
                    continue;
                };
                let (source, knockout) = match &shadow.source {
                    ShadowSource::Content if plan.reuse_content => (content, None),
                    ShadowSource::Content => {
                        let found = contents
                            .iter()
                            .find(|(w, _)| *w == plan.source)
                            .map(|(_, v)| *v);
                        let version = match found {
                            Some(version) => version,
                            None => {
                                let version = self.layer_pass(
                                    "conformance shadow content",
                                    LAYER_FORMAT,
                                    plan.source,
                                    |rec, pass| rec.record_ops(pass, &layer.content),
                                )?;
                                contents.push((plan.source, version));
                                version
                            }
                        };
                        (version, None)
                    }
                    ShadowSource::Box(mesh) => {
                        let found = boxes
                            .iter()
                            .find(|(m, w, _)| *m == &**mesh && *w == plan.source)
                            .map(|(_, _, v)| *v);
                        let version = match found {
                            Some(version) => version,
                            None => {
                                let version = self.box_source_pass(mesh, plan.source)?;
                                boxes.push((mesh, plan.source, version));
                                version
                            }
                        };
                        (version, Some(g.map_mesh(mesh)))
                    }
                };
                let low = self.downsample_passes(source, plan.source, plan.factor)?;
                let size = plan.blurred.size();
                let alpha = self.blur_pass(low, size, plan.sigma[0], [1.0, 0.0], true, [1.0; 4])?;
                let color = to_working(&self.plan, shadow.color).rgba;
                let blurred =
                    self.blur_pass(alpha, size, plan.sigma[1], [0.0, 1.0], false, color)?;
                let b = plan.blurred.bounds().shift(shadow.offset);
                colored.push((blurred, g.map_rect([b.x0, b.y0, b.x1, b.y1]), knockout));
            }
            let full = full_rect(&g);
            group = self.layer_pass("conformance shadow stack", LAYER_FORMAT, g, |rec, pass| {
                for (shadow, rect, knockout) in &colored {
                    match knockout {
                        // 상자 shadow: 상자를 stencil 1로 표시하고 0인 곳(상자 밖)에만 그린 뒤 되돌립니다.
                        Some(mesh) => {
                            rec.stencil_mesh(pass, mesh, StencilMode::IncrementWhereEqual);
                            pass.steps.push(Step::StencilReference(0));
                            rec.draw_graph_texture(
                                pass,
                                *shadow,
                                *rect,
                                TextureFilter::Linear,
                                StencilMode::TestEqual,
                            )?;
                            rec.stencil_mesh(pass, mesh, StencilMode::DecrementWhereEqual);
                        }
                        None => rec.draw_graph_texture(
                            pass,
                            *shadow,
                            *rect,
                            TextureFilter::Linear,
                            StencilMode::Disabled,
                        )?,
                    }
                }
                rec.draw_graph_texture(
                    pass,
                    content,
                    full,
                    TextureFilter::Nearest,
                    StencilMode::Disabled,
                )
            })?;
        }
        if let Some(blur) = windows.blur {
            let low = self.downsample_passes(group, g, blur.factor)?;
            let size = blur.blurred.size();
            let horizontal =
                self.blur_pass(low, size, blur.sigma[0], [1.0, 0.0], false, [1.0; 4])?;
            group = self.blur_pass(horizontal, size, blur.sigma[1], [0.0, 1.0], false, [1.0; 4])?;
            if blur.resolve {
                let blurred = group;
                let rect = parent_window.rect_of(&blur.blurred);
                group = self.layer_pass(
                    "conformance upsample",
                    LAYER_FORMAT,
                    parent_window,
                    |rec, pass| {
                        rec.draw_graph_texture(
                            pass,
                            blurred,
                            rect,
                            TextureFilter::Linear,
                            StencilMode::Disabled,
                        )
                    },
                )?;
            }
        }
        let mask = match (&layer.clip, layer.mask.is_empty()) {
            (Some(ClipSpec::Mask(shapes)), _) => Some(self.mask_pass(shapes, parent_window)?),
            (_, false) => Some(self.mask_pass(&layer.mask, parent_window)?),
            _ => None,
        };
        self.builder.read(parent.id, group)?;
        if let Some(mask) = mask {
            self.builder.read(parent.id, mask)?;
        }
        let stencil_clip = match &layer.clip {
            Some(ClipSpec::Stencil(mesh)) => Some(parent_window.map_mesh(mesh)),
            _ => None,
        };
        let stencil = if let Some([x, y, width, height]) = scissor {
            parent.steps.push(Step::Scissor(Some(ScissorRect {
                x,
                y,
                width,
                height,
            })));
            StencilMode::Disabled
        } else if let Some(mesh) = &stencil_clip {
            parent.stencil = true;
            let (first, count) = self.solid_vertices(&mesh.positions, [0.0; 4]);
            let (uniform_offset, uniform_size) = self.tint([1.0; 4]);
            let draw = Step::Draw {
                stream: Stream::Solid,
                first,
                count,
                uniform_offset,
                uniform_size,
            };
            parent.steps.push(Step::Pipeline(key(
                ShaderProgram::ClipStencil,
                BlendState::Replace,
                StencilMode::IncrementWhereEqual,
                parent.format,
            )));
            parent.steps.push(Step::StencilReference(0));
            parent.steps.push(draw);
            parent.steps.push(Step::StencilReference(1));
            StencilMode::TestEqual
        } else {
            StencilMode::Disabled
        };
        self.composite(parent, group, layer.blend, layer.opacity, mask, stencil);
        if scissor.is_some() {
            parent.steps.push(Step::Scissor(None));
        } else if let Some(mesh) = &stencil_clip {
            let (first, count) = self.solid_vertices(&mesh.positions, [0.0; 4]);
            let (uniform_offset, uniform_size) = self.tint([1.0; 4]);
            parent.steps.push(Step::Pipeline(key(
                ShaderProgram::ClipStencil,
                BlendState::Replace,
                StencilMode::DecrementWhereEqual,
                parent.format,
            )));
            parent.steps.push(Step::StencilReference(0));
            parent.steps.push(Step::Draw {
                stream: Stream::Solid,
                first,
                count,
                uniform_offset,
                uniform_size,
            });
        }
        Ok(())
    }

    /// `window` 크기 transient에 그리는 pass를 기록하고 생산한 version을 돌려줍니다(투명 clear). pass 안 그리기는 device
    /// 좌표이고 [`Recorder::record_ops`]가 window texel 좌표로 옮깁니다.
    fn layer_pass(
        &mut self,
        label: &'static str,
        format: TextureFormat,
        window: Window,
        record: impl FnOnce(&mut Recorder, &mut PassRecord) -> Result<(), LowerError>,
    ) -> Result<ResourceVersion, LowerError> {
        let texture = self.builder.transient(
            label,
            TextureDesc {
                format,
                size: AttachmentSize::Fixed {
                    width: window.width,
                    height: window.height,
                },
                samples: SampleCount::X1,
            },
        );
        let id = self.builder.pass(label, PassKind::Raster);
        let mut pass = PassRecord {
            id,
            format,
            stencil: false,
            window,
            steps: Vec::new(),
        };
        record(self, &mut pass)?;
        let written = self.builder.color(
            id,
            texture,
            LoadOp::Clear,
            crate::render_graph::ClearColor::TRANSPARENT,
            StoreOp::Store,
        )?;
        self.attach_stencil(&pass)?;
        self.records.push(pass);
        Ok(written)
    }

    /// graph texture를 `rect`에 premultiplied over로 그립니다(`LayerComposite`, uv 0..1, `filter`로 읽음, `stencil`
    /// 판정). 1:1 정수 자리면 Nearest·Linear 결과가 같습니다.
    fn draw_graph_texture(
        &mut self,
        pass: &mut PassRecord,
        source: ResourceVersion,
        rect: [f32; 4],
        filter: TextureFilter,
        stencil: StencilMode,
    ) -> Result<(), LowerError> {
        self.draw_graph_quad(pass, source, rect, [0.0, 0.0, 1.0, 1.0], filter, stencil)
    }

    /// graph texture를 `rect`·`uv` quad로 premultiplied over합니다(`LayerComposite`).
    fn draw_graph_quad(
        &mut self,
        pass: &mut PassRecord,
        source: ResourceVersion,
        rect: [f32; 4],
        uv: [f32; 4],
        filter: TextureFilter,
        stencil: StencilMode,
    ) -> Result<(), LowerError> {
        self.builder.read(pass.id, source)?;
        let (first, count) = self.textured_quad(rect, uv);
        let (uniform_offset, uniform_size) = self.tint([1.0; 4]);
        pass.steps.push(Step::Pipeline(key(
            ShaderProgram::LayerComposite,
            BlendState::PremultipliedOver,
            stencil,
            pass.format,
        )));
        pass.steps.push(Step::Texture {
            slot: 0,
            texture: TexRef::Graph(source),
            sampler: clamp_sampler(filter),
        });
        pass.steps.push(Step::Draw {
            stream: Stream::Textured,
            first,
            count,
            uniform_offset,
            uniform_size,
        });
        Ok(())
    }

    /// mesh를 stencil에 그립니다(`ClipStencil`, reference 0). pass에 stencil attachment를 붙입니다.
    fn stencil_mesh(&mut self, pass: &mut PassRecord, mesh: &Mesh, mode: StencilMode) {
        pass.stencil = true;
        let (first, count) = self.solid_vertices(&mesh.positions, [0.0; 4]);
        let (uniform_offset, uniform_size) = self.tint([1.0; 4]);
        pass.steps.push(Step::Pipeline(key(
            ShaderProgram::ClipStencil,
            BlendState::Replace,
            mode,
            pass.format,
        )));
        pass.steps.push(Step::StencilReference(0));
        pass.steps.push(Step::Draw {
            stream: Stream::Solid,
            first,
            count,
            uniform_offset,
            uniform_size,
        });
    }

    /// 상자 shadow 원본: mesh를 불투명 흰색으로 칠한 `window` layer(ADR-0130 결정 1).
    fn box_source_pass(
        &mut self,
        mesh: &Mesh,
        window: Window,
    ) -> Result<ResourceVersion, LowerError> {
        self.layer_pass(
            "conformance box shadow",
            LAYER_FORMAT,
            window,
            |rec, pass| {
                if mesh.positions.is_empty() {
                    return Ok(());
                }
                let local = window.map_mesh(mesh);
                let (first, count) = rec.solid_vertices(&local.positions, [1.0; 4]);
                let (uniform_offset, uniform_size) = rec.tint([1.0; 4]);
                pass.steps.push(Step::Pipeline(key(
                    ShaderProgram::SolidFill,
                    BlendState::PremultipliedOver,
                    StencilMode::Disabled,
                    pass.format,
                )));
                pass.steps.push(Step::Draw {
                    stream: Stream::Solid,
                    first,
                    count,
                    uniform_offset,
                    uniform_size,
                });
                Ok(())
            },
        )
    }

    /// `source`(`window` 크기)를 `d`배 줄입니다(ADR-0130 결정 2). 한 단계마다 `ceil(n / 2)` 크기 pass에 bilinear 표본 하나
    /// (2×2 상자 평균, 가장자리 clamp)입니다. 결과 version을 돌려줍니다(크기는 `window.downsampled(d)`).
    fn downsample_passes(
        &mut self,
        source: ResourceVersion,
        window: Window,
        d: u32,
    ) -> Result<ResourceVersion, LowerError> {
        let mut current = source;
        let mut level = window;
        let mut d = d.max(1);
        while d > 1 {
            let next = level.downsampled(2);
            let (width, height) = level.size();
            let (nw, nh) = next.size();
            let uv = [
                0.0,
                0.0,
                (2 * nw) as f32 / width as f32,
                (2 * nh) as f32 / height as f32,
            ];
            let input = current;
            current =
                self.layer_pass("conformance downsample", LAYER_FORMAT, next, |rec, pass| {
                    rec.draw_graph_quad(
                        pass,
                        input,
                        [0.0, 0.0, nw as f32, nh as f32],
                        uv,
                        TextureFilter::Linear,
                        StencilMode::Disabled,
                    )
                })?;
            level = next;
            d /= 2;
        }
        Ok(current)
    }

    /// 한 방향 gaussian blur pass(`GaussianBlur`, ADR-0102 결정 5). source와 같은 `size`입니다.
    fn blur_pass(
        &mut self,
        source: ResourceVersion,
        size: (u32, u32),
        sigma: f32,
        direction: [f32; 2],
        source_alpha: bool,
        tint: [f32; 4],
    ) -> Result<ResourceVersion, LowerError> {
        // blur pass는 texel 좌표만 씁니다(window 원점·해상도는 그리기 좌표에 쓰이지 않음).
        let window = Window::full(size.0, size.1);
        self.layer_pass("conformance blur", LAYER_FORMAT, window, |rec, pass| {
            rec.builder.read(pass.id, source)?;
            let full = [0.0, 0.0, size.0 as f32, size.1 as f32];
            let (first, count) = rec.textured_quad(full, [0.0, 0.0, 1.0, 1.0]);
            let (uniform_offset, uniform_size) =
                rec.uniform(blur_uniforms(tint, sigma, direction, source_alpha));
            pass.steps.push(Step::Pipeline(key(
                ShaderProgram::GaussianBlur,
                BlendState::Replace,
                StencilMode::Disabled,
                pass.format,
            )));
            pass.steps.push(Step::Texture {
                slot: 0,
                texture: TexRef::Graph(source),
                sampler: clamp_sampler(TextureFilter::Nearest),
            });
            pass.steps.push(Step::Draw {
                stream: Stream::Textured,
                first,
                count,
                uniform_offset,
                uniform_size,
            });
            Ok(())
        })
    }

    /// coverage 도형을 `R8Unorm` mask에 합집합으로 그립니다(`ClipMask` + premultiplied over).
    fn mask_pass(
        &mut self,
        shapes: &[CoverageShape],
        window: Window,
    ) -> Result<ResourceVersion, LowerError> {
        self.layer_pass(
            "conformance mask",
            TextureFormat::R8Unorm,
            window,
            |rec, pass| {
                for shape in shapes {
                    if shape.mesh.positions.is_empty() {
                        continue;
                    }
                    let mesh = window.map_mesh(&shape.mesh);
                    let (first, count) = rec.solid_vertices(&mesh.positions, [0.0; 4]);
                    let (uniform_offset, uniform_size) = rec.tint([shape.coverage; 4]);
                    pass.steps.push(Step::Pipeline(key(
                        ShaderProgram::ClipMask,
                        BlendState::PremultipliedOver,
                        StencilMode::Disabled,
                        pass.format,
                    )));
                    pass.steps.push(Step::Draw {
                        stream: Stream::Solid,
                        first,
                        count,
                        uniform_offset,
                        uniform_size,
                    });
                }
                Ok(())
            },
        )
    }

    /// graph texture 전체를 부모 target에 합성합니다: 불투명도 tint, mask(slot 1), blend program(ADR-0102 결정 3·5).
    fn composite(
        &mut self,
        parent: &mut PassRecord,
        source: ResourceVersion,
        mode: SceneBlendMode,
        opacity: f32,
        mask: Option<ResourceVersion>,
        stencil: StencilMode,
    ) {
        let full = full_rect(&parent.window);
        let (first, count) = self.textured_quad(full, [0.0, 0.0, 1.0, 1.0]);
        let (uniform_offset, uniform_size) = self.uniform(draw_uniforms(
            [opacity; 4],
            DEFAULT_DEPTH,
            if mask.is_some() {
                MODE_MASKED
            } else {
                MODE_PLAIN
            },
            [0.0; 2],
        ));
        parent.steps.push(Step::Pipeline(key(
            composite_program(mode),
            composite_blend_state(mode),
            stencil,
            parent.format,
        )));
        parent.steps.push(Step::Texture {
            slot: 0,
            texture: TexRef::Graph(source),
            sampler: clamp_sampler(TextureFilter::Nearest),
        });
        if let Some(mask) = mask {
            parent.steps.push(Step::Texture {
                slot: 1,
                texture: TexRef::Graph(mask),
                sampler: clamp_sampler(TextureFilter::Nearest),
            });
        }
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
                TextureDesc {
                    format: TextureFormat::Depth32FloatStencil8,
                    size: AttachmentSize::Fixed {
                        width: pass.window.width,
                        height: pass.window.height,
                    },
                    samples: SampleCount::X1,
                },
            );
            self.builder
                .depth_stencil(pass.id, depth, LoadOp::Clear, StoreOp::Discard)?;
        }
        Ok(())
    }

    fn record_fill(&mut self, pass: &mut PassRecord, fill: &FillOp) -> Result<(), LowerError> {
        if fill.mesh.positions.is_empty() {
            return Ok(());
        }
        let window = pass.window;
        let mesh = window.map_mesh(&fill.mesh);
        let (uniform_offset, uniform_size) = self.tint([1.0; 4]);
        let solid_draw = |first, count| Step::Draw {
            stream: Stream::Solid,
            first,
            count,
            uniform_offset,
            uniform_size,
        };
        // (program, 칠하기 draw, ramp texture, stencil을 0으로 되돌리는 draw)
        let (program, draw, texture, restore) = match &fill.paint {
            Paint::Solid(color) => {
                let color = to_working(&self.plan, *color).rgba;
                let (first, count) = self.solid_vertices(&mesh.positions, color);
                let draw = solid_draw(first, count);
                (ShaderProgram::SolidFill, draw, None, draw)
            }
            Paint::Gradient { geometry, stops } => {
                if !geometry.is_affine() {
                    return Err(LowerError::Unsupported(
                        "원형 gradient는 GPU 경로가 없습니다(ADR-0102 결정 4)",
                    ));
                }
                let ramp = GradientRamp::resolve(&self.plan, stops)
                    .map_err(|error| LowerError::Fixture(FixtureError::Gradient(error)))?;
                let bytes = ramp
                    .ramp(RAMP_WIDTH)
                    .into_iter()
                    .flat_map(|c| {
                        encode_pixel(c, TextureFormat::Rgba16Float, self.plan.surface())
                            .expect("float format")
                    })
                    .collect();
                let upload = self.upload(RAMP_WIDTH, 1, TextureFormat::Rgba16Float, bytes);
                // gradient 매개변수는 device 좌표의 affine 함수라 정점에서 계산합니다.
                let vertices: Vec<([f32; 2], [f32; 2])> = fill
                    .mesh
                    .positions
                    .iter()
                    .map(|p| {
                        (
                            window.map(*p),
                            [ramp.ramp_coordinate(geometry.parameter(*p)), 0.5],
                        )
                    })
                    .collect();
                let (first, count) = self.textured_vertices(&vertices);
                let (restore_first, restore_count) = self.solid_vertices(&mesh.positions, [0.0; 4]);
                (
                    ShaderProgram::Texture,
                    Step::Draw {
                        stream: Stream::Textured,
                        first,
                        count,
                        uniform_offset,
                        uniform_size,
                    },
                    Some(Step::Texture {
                        slot: 0,
                        texture: TexRef::Upload(upload),
                        sampler: clamp_sampler(TextureFilter::Linear),
                    }),
                    solid_draw(restore_first, restore_count),
                )
            }
        };
        let stencil = match fill.coverage {
            Coverage::Disjoint => StencilMode::Disabled,
            Coverage::Union => StencilMode::IncrementWhereEqual,
        };
        pass.steps.push(Step::Pipeline(key(
            program,
            BlendState::PremultipliedOver,
            stencil,
            pass.format,
        )));
        if fill.coverage == Coverage::Union {
            pass.stencil = true;
            pass.steps.push(Step::StencilReference(0));
        }
        pass.steps.extend(texture);
        pass.steps.push(draw);
        if fill.coverage == Coverage::Union {
            pass.steps.push(Step::Pipeline(key(
                ShaderProgram::ClipStencil,
                BlendState::Replace,
                StencilMode::DecrementWhereEqual,
                pass.format,
            )));
            pass.steps.push(restore);
        }
        Ok(())
    }

    /// 임의 uv 정점 삼각형 목록.
    pub(crate) fn textured_vertices(&mut self, vertices: &[([f32; 2], [f32; 2])]) -> (u32, u32) {
        let first = (self.textured.len() / 16) as u32;
        for (p, t) in vertices {
            push_textured_vertex(&mut self.textured, *p, *t);
        }
        (first, vertices.len() as u32)
    }

    fn record_image(&mut self, pass: &mut PassRecord, image: &ImageOp) {
        let data = &image.image;
        let upload = self.upload(
            data.width,
            data.height,
            IMAGE_TEXTURE_FORMAT,
            data.texture_bytes(&self.plan),
        );
        let (first, count) = self.textured_quad(pass.window.map_rect(image.rect), image.uv);
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
            sampler: SamplerDesc {
                filter: image.filter,
                wrap: image.wrap,
                mipmaps: false,
            },
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
        // mask는 device 정수 원점에 1:1입니다. 해상도가 1이 아닌 window에서는 Nearest로 읽습니다(CPU 기준과 같음).
        let (first, count) = self.textured_quad(
            pass.window
                .map_rect([ox, oy, ox + mask.width as f32, oy + mask.height as f32]),
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
    lower_ops(
        backend,
        &OpsTarget {
            plan: fixture.plan(),
            width: fixture.width,
            height: fixture.height,
            format: fixture.format,
            clear: fixture.clear_working(),
            origin: [0, 0],
            space: EffectSpace::full(fixture.width, fixture.height),
        },
        &fixture.ops,
    )
}

/// 그리기 목록 target(fixture 크기·format 제한 없음). 제품 offscreen 렌더(ADR-0116 결정 4)가 씁니다.
#[derive(Debug, Clone, Copy)]
pub struct OpsTarget {
    /// 출력 색 변환 규칙(working space = surface 색공간).
    pub plan: OutputColorPlan,
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
    /// target format(sampled + render target).
    pub format: TextureFormat,
    /// clear 색(working).
    pub clear: crate::color::WorkingColor,
    /// target 좌상단의 device 좌표(ops 좌표 기준). fixture는 원점 0입니다.
    pub origin: [i64; 2],
    /// ops 공간(효과 window의 device 범위와 한 변 상한, ADR-0136).
    pub space: EffectSpace,
}

/// 그리기 목록을 `target` 하나에 그리는 frame으로 바꿉니다. 값 계약은 fixture와 같습니다.
pub fn lower_ops<B: RendererBackend + ?Sized>(
    backend: &mut B,
    target_desc: &OpsTarget,
    ops: &[Op],
) -> Result<LoweredFrame, LowerError> {
    super::fixture::validate_ops(ops, &target_desc.plan)?;
    if target_desc.width == 0 || target_desc.height == 0 {
        return Err(LowerError::Fixture(FixtureError::InvalidSize));
    }
    let mut recorder = Recorder::new(
        target_desc.plan,
        target_desc.width,
        target_desc.height,
        target_desc.space,
    );
    let target = recorder.builder.import(
        "conformance target",
        recorder.desc(target_desc.format),
        ImportKind::External,
    );
    let main = recorder.builder.pass("conformance main", PassKind::Raster);
    let mut record = PassRecord {
        id: main,
        format: target_desc.format,
        stencil: false,
        window: Window {
            x: target_desc.origin[0],
            y: target_desc.origin[1],
            width: target_desc.width,
            height: target_desc.height,
            step: 1,
        },
        steps: Vec::new(),
    };
    recorder.record_ops(&mut record, ops)?;
    let clear = target_desc
        .clear
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
    finish(
        backend,
        (target_desc.width, target_desc.height, target_desc.format),
        recorder,
        graph,
    )
}

/// buffer·texture를 만들고 step을 command로 바꿉니다.
pub(crate) fn finish<B: RendererBackend + ?Sized>(
    backend: &mut B,
    (target_width, target_height, target_format): (u32, u32, TextureFormat),
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
                width: target_width,
                height: target_height,
                format: target_format,
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
                        Step::Scissor(rect) => commands.push(DrawCommand::SetScissor(rect)),
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

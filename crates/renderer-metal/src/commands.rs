//! pass [`DrawCommand`] → `MTLRenderCommandEncoder` 호출 (XI.8.3, texture·depth-stencil XI.8.4).
//!
//! [`plan_commands`]가 command 목록을 상태 기계로 따라가며 handle을 native 객체로 해석하고, GPU가
//! buffer 밖을 읽지 않도록 범위를 모두 검사한 [`Op`] 목록을 만듭니다. 검사는 encoder를 열기 전에
//! 끝나므로([`crate::encode`] 2단계) 잘못된 command는 command buffer에 아무것도 남기지 않습니다.
//!
//! - draw 전에는 pipeline·vertex buffer·uniform이 bind되어 있어야 합니다(`DrawIndexed`는 index buffer도).
//! - `Draw`는 `vertex_start + vertex_count`가 bind한 vertex buffer 용량(stride 기준) 안이어야 합니다.
//! - `DrawIndexed`는 index 범위가 index buffer 안이어야 하고, CPU에서 그 범위의 index를 읽어
//!   `base_vertex + index`가 모두 vertex 용량 안인지 확인합니다(index 수에 비례하는 CPU 비용).
//! - scissor는 attachment 크기로 자릅니다. 잘린 결과가 비면 다음 scissor 변경까지 draw를 건너뜁니다.
//! - vertex 수·instance 수가 0인 draw는 건너뜁니다.
//! - `LayerComposite`·`BlendComposite`는 slot 1 mask를 선언합니다. slot 1이 비어 있으면 draw 직전에 slot 0 texture를 함께
//!   bind합니다(shader는 `DrawUniforms.mode`가 1일 때만 읽음). `BlendComposite`는 shader가 합성하므로 blend state가
//!   `Replace`가 아니면 `InvalidUsage`입니다. uniform 범위는 program uniform 크기 이상이어야 합니다.
//! - sampling program(`Texture`·`LayerComposite`·`GlyphMask`)은 draw 전에 texture slot 0이 있어야
//!   합니다. bind하는 texture는 sampled 용도·single sample·color format이어야 하고, 이 pass가 그리는
//!   attachment texture가 아니어야 합니다(feedback loop).
//! - depth·stencil attachment가 있는 pass는 pipeline마다 depth-stencil state도 bind합니다.
//!   `DecrementWhereEqual`은 "reference + 1과 같은 값"을 비교하므로 backend가 Metal reference를 1 올려
//!   넣습니다(reference 255는 비교 대상이 없어 `InvalidUsage`).

use stellar_renderer_core::backend::{
    BackendError, BackendErrorKind, BlendState, BufferHandle, BufferUsage, DrawCommand,
    PipelineKey, StencilMode, TextureSource,
};
use stellar_renderer_core::gpu_handle::{BufferKind, HandleTable, TextureKind};
use stellar_renderer_core::render_graph::{ResourceVersion, SampleCount, TextureFormat};

use crate::buffer::MetalBuffer;
use crate::objc::{sel, send, Id, NSUInteger, RawId};
use crate::pipeline::PipelineCache;
use crate::shaders::{
    program_layout, ProgramLayout, DRAW_UNIFORMS_SIZE, MAX_TEXTURE_SLOTS, UNIFORM_OFFSET_ALIGNMENT,
    VERTEX_OFFSET_ALIGNMENT,
};
use crate::sys::{MTLScissorRect, INDEX_TYPE_UINT32, PRIMITIVE_TRIANGLE};
use crate::texture::MetalTexture;

fn invalid(detail: &'static str) -> BackendError {
    BackendError::new(BackendErrorKind::InvalidUsage, detail)
}

/// draw가 그리는 pass attachment 정보.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PassFormat {
    /// color attachment 0 format.
    pub(crate) color_format: TextureFormat,
    /// sample 수.
    pub(crate) samples: SampleCount,
    /// color attachment 수.
    pub(crate) color_count: usize,
    /// attachment 너비.
    pub(crate) width: u32,
    /// attachment 높이.
    pub(crate) height: u32,
    /// depth·stencil attachment format(없으면 `None`).
    pub(crate) depth_format: Option<TextureFormat>,
}

/// command 해석 문맥.
pub(crate) struct CommandContext<'a> {
    /// `id<MTLDevice>`.
    pub(crate) device: &'a Id,
    /// backend buffer table.
    pub(crate) buffers: &'a HandleTable<BufferKind, MetalBuffer>,
    /// backend texture table.
    pub(crate) textures: &'a HandleTable<TextureKind, MetalTexture>,
    /// pipeline cache.
    pub(crate) pipelines: &'a mut PipelineCache,
}

/// 검사를 마친 encoder 호출. native 포인터는 table·cache가 소유합니다(+0).
///
/// draw 수는 [`Op::is_draw`]로 셉니다(frame trace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    /// `setRenderPipelineState:`.
    Pipeline(RawId),
    /// `setDepthStencilState:`.
    DepthStencil(RawId),
    /// fragment texture·sampler slot.
    Texture {
        /// slot.
        slot: NSUInteger,
        /// `id<MTLTexture>`.
        texture: RawId,
        /// `id<MTLSamplerState>`.
        sampler: RawId,
    },
    /// `setScissorRect:`.
    Scissor(MTLScissorRect),
    /// `setStencilReferenceValue:`.
    StencilReference(u32),
    /// vertex buffer slot 0.
    VertexBuffer {
        /// buffer.
        buffer: RawId,
        /// byte offset.
        offset: NSUInteger,
    },
    /// uniform slot 1(vertex·fragment).
    Uniforms {
        /// buffer.
        buffer: RawId,
        /// byte offset.
        offset: NSUInteger,
    },
    /// `drawPrimitives:vertexStart:vertexCount:instanceCount:`.
    Draw {
        /// 첫 vertex.
        start: NSUInteger,
        /// vertex 수.
        count: NSUInteger,
        /// instance 수.
        instances: NSUInteger,
    },
    /// `drawIndexedPrimitives:...`.
    DrawIndexed {
        /// index 수.
        count: NSUInteger,
        /// index buffer.
        buffer: RawId,
        /// index buffer byte offset(첫 index 포함).
        offset: NSUInteger,
        /// vertex index에 더할 값.
        base_vertex: isize,
        /// instance 수.
        instances: NSUInteger,
    },
}

impl Op {
    /// draw 호출이면 참입니다.
    pub(crate) const fn is_draw(&self) -> bool {
        matches!(self, Op::Draw { .. } | Op::DrawIndexed { .. })
    }
}

#[derive(Debug, Default)]
struct DrawState {
    pipeline: Option<(PipelineKey, ProgramLayout)>,
    vertex: Option<(BufferHandle, u64)>,
    index: Option<(BufferHandle, u64)>,
    /// bind한 uniform 범위 byte 수(없으면 `None`).
    uniforms: Option<u32>,
    /// slot별 bind한 (texture, sampler).
    textures: [Option<(RawId, RawId)>; MAX_TEXTURE_SLOTS as usize],
    scissor_empty: bool,
    reference: u8,
    emitted_reference: Option<u32>,
}

impl DrawState {
    /// draw 직전 공통 검사. pipeline layout을 돌려줍니다.
    fn ready(&self) -> Result<ProgramLayout, BackendError> {
        let (_, layout) = self
            .pipeline
            .ok_or(invalid("draw 전에 pipeline을 bind해야 합니다"))?;
        let uniforms = self
            .uniforms
            .ok_or(invalid("draw 전에 uniform을 bind해야 합니다"))?;
        if uniforms < layout.uniform_size {
            return Err(invalid("uniform 범위가 program uniform 크기보다 작습니다"));
        }
        if layout.needs_texture && self.textures[0].is_none() {
            return Err(invalid("sampling program은 texture slot 0이 필요합니다"));
        }
        Ok(layout)
    }

    /// mask slot을 선언한 program인데 slot 1이 비어 있으면 slot 0 texture를 함께 bind합니다(shader는 mode가 1일 때만 읽음).
    fn fill_mask_slot(&mut self, layout: &ProgramLayout, ops: &mut Vec<Op>) {
        if !layout.mask_slot || self.textures[1].is_some() {
            return;
        }
        if let Some((texture, sampler)) = self.textures[0] {
            self.textures[1] = Some((texture, sampler));
            ops.push(Op::Texture {
                slot: 1,
                texture,
                sampler,
            });
        }
    }

    /// 현재 pipeline stencil mode에 맞춘 Metal stencil reference.
    fn effective_reference(&self) -> Result<u32, BackendError> {
        match self.pipeline.map(|(key, _)| key.stencil) {
            Some(StencilMode::DecrementWhereEqual) => {
                if self.reference == u8::MAX {
                    return Err(invalid(
                        "stencil reference 255는 감소 비교를 할 수 없습니다",
                    ));
                }
                Ok(u32::from(self.reference) + 1)
            }
            _ => Ok(u32::from(self.reference)),
        }
    }

    /// depth·stencil attachment가 있으면 reference가 바뀔 때만 Op를 냅니다.
    fn sync_reference(&mut self, pass: &PassFormat, ops: &mut Vec<Op>) -> Result<(), BackendError> {
        if pass.depth_format.is_none() {
            return Ok(());
        }
        let effective = self.effective_reference()?;
        if self.emitted_reference != Some(effective) {
            ops.push(Op::StencilReference(effective));
            self.emitted_reference = Some(effective);
        }
        Ok(())
    }
}

/// bind한 vertex buffer에 들어가는 vertex 수.
fn vertex_capacity(
    ctx: &CommandContext<'_>,
    state: &DrawState,
    layout: &ProgramLayout,
) -> Result<u64, BackendError> {
    let (handle, offset) = state.vertex.ok_or(invalid("vertex buffer가 없습니다"))?;
    let buffer = ctx.buffers.get(handle)?;
    Ok(buffer.desc.size.saturating_sub(offset) / u64::from(layout.stride))
}

/// command 목록을 검사하고 encoder 호출로 바꿉니다.
///
/// `attachments`는 이 pass가 그리는 texture(+0), `graph_reads`는 이 pass가 읽는 graph version과
/// sampling 가능한 texture(+0, 불가면 `None`)입니다.
pub(crate) fn plan_commands(
    ctx: &mut CommandContext<'_>,
    pass: &PassFormat,
    attachments: &[RawId],
    graph_reads: &[(ResourceVersion, Option<RawId>)],
    commands: &[DrawCommand],
) -> Result<Vec<Op>, BackendError> {
    let mut ops = Vec::with_capacity(commands.len());
    let mut state = DrawState::default();
    for command in commands {
        match *command {
            DrawCommand::SetPipeline(key) => {
                if pass.color_count != 1 {
                    return Err(BackendError::new(
                        BackendErrorKind::Unsupported,
                        "draw는 color attachment 하나인 pass에서만 지원합니다",
                    ));
                }
                if key.color_format != pass.color_format || key.samples != pass.samples {
                    return Err(invalid(
                        "pipeline format·sample 수가 pass attachment와 다릅니다",
                    ));
                }
                let layout = program_layout(key.program).ok_or(BackendError::new(
                    BackendErrorKind::Unsupported,
                    "Metal backend가 아직 구현하지 않은 shader program",
                ))?;
                if layout.shader_blend && key.blend != BlendState::Replace {
                    return Err(invalid(
                        "BlendComposite는 shader가 합성하므로 blend state가 Replace여야 합니다",
                    ));
                }
                let pipeline = ctx
                    .pipelines
                    .pipeline(ctx.device, &key, pass.depth_format)?;
                ops.push(Op::Pipeline(pipeline));
                if pass.depth_format.is_some() {
                    let depth = ctx
                        .pipelines
                        .depth_stencil(ctx.device, key.depth, key.stencil)?;
                    ops.push(Op::DepthStencil(depth));
                }
                state.pipeline = Some((key, layout));
                state.sync_reference(pass, &mut ops)?;
            }
            DrawCommand::SetScissor(rect) => {
                let (x0, y0, x1, y1) = match rect {
                    None => (0, 0, pass.width, pass.height),
                    Some(rect) => (
                        rect.x.min(pass.width),
                        rect.y.min(pass.height),
                        rect.x.saturating_add(rect.width).min(pass.width),
                        rect.y.saturating_add(rect.height).min(pass.height),
                    ),
                };
                state.scissor_empty = x1 <= x0 || y1 <= y0;
                if !state.scissor_empty {
                    ops.push(Op::Scissor(MTLScissorRect {
                        x: x0 as NSUInteger,
                        y: y0 as NSUInteger,
                        width: (x1 - x0) as NSUInteger,
                        height: (y1 - y0) as NSUInteger,
                    }));
                }
            }
            DrawCommand::SetStencilReference(reference) => {
                state.reference = reference;
                state.sync_reference(pass, &mut ops)?;
            }
            DrawCommand::SetVertexBuffer { buffer, offset } => {
                let native = ctx.buffers.get(buffer)?;
                native.expect_usage(BufferUsage::Vertex)?;
                if offset % VERTEX_OFFSET_ALIGNMENT != 0 || offset >= native.desc.size {
                    return Err(invalid("vertex buffer offset이 정렬·범위를 벗어났습니다"));
                }
                state.vertex = Some((buffer, offset));
                ops.push(Op::VertexBuffer {
                    buffer: native.buffer.as_raw(),
                    offset: offset as NSUInteger,
                });
            }
            DrawCommand::SetIndexBuffer { buffer, offset } => {
                let native = ctx.buffers.get(buffer)?;
                native.expect_usage(BufferUsage::Index)?;
                if offset % VERTEX_OFFSET_ALIGNMENT != 0 || offset > native.desc.size {
                    return Err(invalid("index buffer offset이 정렬·범위를 벗어났습니다"));
                }
                state.index = Some((buffer, offset));
            }
            DrawCommand::SetUniforms {
                buffer,
                offset,
                size,
            } => {
                let native = ctx.buffers.get(buffer)?;
                native.expect_usage(BufferUsage::Uniform)?;
                let end = offset.checked_add(u64::from(size));
                if offset % UNIFORM_OFFSET_ALIGNMENT != 0
                    || size < DRAW_UNIFORMS_SIZE
                    || end.is_none_or(|end| end > native.desc.size)
                {
                    return Err(invalid(
                        "uniform 범위가 정렬·크기·buffer 범위를 벗어났습니다",
                    ));
                }
                state.uniforms = Some(size);
                ops.push(Op::Uniforms {
                    buffer: native.buffer.as_raw(),
                    offset: offset as NSUInteger,
                });
            }
            DrawCommand::BindTexture {
                slot,
                texture,
                sampler,
            } => {
                if slot >= MAX_TEXTURE_SLOTS {
                    return Err(invalid("texture slot 범위 밖"));
                }
                let texture = match texture {
                    TextureSource::Handle(handle) => {
                        let native = ctx.textures.get(handle)?;
                        let desc = native.desc;
                        if !desc.usage.sampled
                            || desc.samples != SampleCount::X1
                            || desc.format.is_depth_stencil()
                        {
                            return Err(invalid("sampling할 수 없는 texture입니다"));
                        }
                        let texture = native.texture.as_raw();
                        if attachments.contains(&texture) {
                            return Err(invalid(
                                "pass가 그리는 attachment texture는 sampling할 수 없습니다",
                            ));
                        }
                        texture
                    }
                    TextureSource::Graph(version) => graph_reads
                        .iter()
                        .find(|(read, _)| *read == version)
                        .ok_or(invalid("pass가 읽는다고 선언하지 않은 graph version"))?
                        .1
                        .ok_or(invalid("sampling할 수 없는 graph resource입니다"))?,
                };
                let sampler = ctx.pipelines.sampler(ctx.device, sampler)?;
                state.textures[usize::from(slot)] = Some((texture, sampler));
                ops.push(Op::Texture {
                    slot: NSUInteger::from(slot),
                    texture,
                    sampler,
                });
            }
            DrawCommand::Draw {
                vertex_start,
                vertex_count,
                instance_count,
            } => {
                let layout = state.ready()?;
                state.fill_mask_slot(&layout, &mut ops);
                let capacity = vertex_capacity(ctx, &state, &layout)?;
                if u64::from(vertex_start) + u64::from(vertex_count) > capacity {
                    return Err(invalid("vertex 범위가 vertex buffer 밖입니다"));
                }
                if vertex_count == 0 || instance_count == 0 || state.scissor_empty {
                    continue;
                }
                ops.push(Op::Draw {
                    start: vertex_start as NSUInteger,
                    count: vertex_count as NSUInteger,
                    instances: instance_count as NSUInteger,
                });
            }
            DrawCommand::DrawIndexed {
                index_start,
                index_count,
                base_vertex,
                instance_count,
            } => {
                let layout = state.ready()?;
                state.fill_mask_slot(&layout, &mut ops);
                let capacity = vertex_capacity(ctx, &state, &layout)?;
                let (index_handle, index_offset) = state
                    .index
                    .ok_or(invalid("indexed draw 전에 index buffer를 bind해야 합니다"))?;
                let index_buffer = ctx.buffers.get(index_handle)?;
                let first = index_offset
                    .checked_add(u64::from(index_start) * 4)
                    .ok_or(invalid("index 범위가 index buffer 밖입니다"))?;
                let (min, max) = index_buffer
                    .index_bounds(first, index_count)
                    .ok_or(invalid("index 범위가 index buffer 밖입니다"))?;
                if index_count == 0 || instance_count == 0 || state.scissor_empty {
                    continue;
                }
                let lowest = i64::from(base_vertex) + i64::from(min);
                let highest = i64::from(base_vertex) + i64::from(max);
                if lowest < 0 || highest >= i64::try_from(capacity).unwrap_or(i64::MAX) {
                    return Err(invalid("index가 가리키는 vertex가 vertex buffer 밖입니다"));
                }
                ops.push(Op::DrawIndexed {
                    count: index_count as NSUInteger,
                    buffer: index_buffer.buffer.as_raw(),
                    offset: first as NSUInteger,
                    base_vertex: base_vertex as isize,
                    instances: instance_count as NSUInteger,
                });
            }
        }
    }
    Ok(ops)
}

/// 계획한 호출을 encoder에 기록합니다. 계획의 native 객체는 호출 동안 살아 있어야 합니다.
pub(crate) fn record(encoder: RawId, ops: &[Op], width: u32, height: u32) {
    if ops.is_empty() {
        return;
    }
    let target: [f32; 2] = [width as f32, height as f32];
    // SAFETY: render command encoder method 시그니처: setVertexBytes:length:atIndex:(const void*,
    // NSUInteger, NSUInteger), setRenderPipelineState:/setDepthStencilState:(id),
    // setFragmentTexture:atIndex:/setFragmentSamplerState:atIndex:(id, NSUInteger),
    // setScissorRect:(MTLScissorRect),
    // setStencilReferenceValue:(uint32_t), set{Vertex,Fragment}Buffer:offset:atIndex:(id, NSUInteger,
    // NSUInteger), drawPrimitives:vertexStart:vertexCount:instanceCount:(NSUInteger ×4),
    // drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:instanceCount:
    // baseVertex:baseInstance:(NSUInteger, NSUInteger, NSUInteger, id, NSUInteger, NSUInteger,
    // NSInteger, NSUInteger). 모든 범위는 plan_commands가 검사했습니다.
    unsafe {
        let _: () = send(
            encoder,
            sel!("setVertexBytes:length:atIndex:"),
            (
                target.as_ptr().cast::<std::ffi::c_void>(),
                std::mem::size_of_val(&target) as NSUInteger,
                2 as NSUInteger,
            ),
        );
        for op in ops {
            match *op {
                Op::Pipeline(state) => {
                    let _: () = send(encoder, sel!("setRenderPipelineState:"), (state,));
                }
                Op::DepthStencil(state) => {
                    let _: () = send(encoder, sel!("setDepthStencilState:"), (state,));
                }
                Op::Texture {
                    slot,
                    texture,
                    sampler,
                } => {
                    let _: () = send(
                        encoder,
                        sel!("setFragmentTexture:atIndex:"),
                        (texture, slot),
                    );
                    let _: () = send(
                        encoder,
                        sel!("setFragmentSamplerState:atIndex:"),
                        (sampler, slot),
                    );
                }
                Op::Scissor(rect) => {
                    let _: () = send(encoder, sel!("setScissorRect:"), (rect,));
                }
                Op::StencilReference(reference) => {
                    let _: () = send(encoder, sel!("setStencilReferenceValue:"), (reference,));
                }
                Op::VertexBuffer { buffer, offset } => {
                    let _: () = send(
                        encoder,
                        sel!("setVertexBuffer:offset:atIndex:"),
                        (buffer, offset, 0 as NSUInteger),
                    );
                }
                Op::Uniforms { buffer, offset } => {
                    let _: () = send(
                        encoder,
                        sel!("setVertexBuffer:offset:atIndex:"),
                        (buffer, offset, 1 as NSUInteger),
                    );
                    let _: () = send(
                        encoder,
                        sel!("setFragmentBuffer:offset:atIndex:"),
                        (buffer, offset, 1 as NSUInteger),
                    );
                }
                Op::Draw {
                    start,
                    count,
                    instances,
                } => {
                    let _: () = send(
                        encoder,
                        sel!("drawPrimitives:vertexStart:vertexCount:instanceCount:"),
                        (PRIMITIVE_TRIANGLE, start, count, instances),
                    );
                }
                Op::DrawIndexed {
                    count,
                    buffer,
                    offset,
                    base_vertex,
                    instances,
                } => {
                    let _: () = send(
                        encoder,
                        sel!("drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:instanceCount:baseVertex:baseInstance:"),
                        (
                            PRIMITIVE_TRIANGLE,
                            count,
                            INDEX_TYPE_UINT32,
                            buffer,
                            offset,
                            instances,
                            base_vertex,
                            0 as NSUInteger,
                        ),
                    );
                }
            }
        }
    }
}

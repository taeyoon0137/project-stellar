//! pass [`DrawCommand`] → `MTLRenderCommandEncoder` 호출 (XI.8.3).
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

use stellar_renderer_core::backend::{
    BackendError, BackendErrorKind, BufferHandle, BufferUsage, DrawCommand, PipelineKey,
};
use stellar_renderer_core::gpu_handle::{BufferKind, HandleTable};
use stellar_renderer_core::render_graph::{SampleCount, TextureFormat};

use crate::buffer::MetalBuffer;
use crate::objc::{sel, send, Id, NSUInteger, RawId};
use crate::pipeline::PipelineCache;
use crate::shaders::{
    vertex_stride, DRAW_UNIFORMS_SIZE, UNIFORM_OFFSET_ALIGNMENT, VERTEX_OFFSET_ALIGNMENT,
};
use crate::sys::{MTLScissorRect, INDEX_TYPE_UINT32, PRIMITIVE_TRIANGLE};

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
}

/// command 해석 문맥.
pub(crate) struct CommandContext<'a> {
    /// `id<MTLDevice>`.
    pub(crate) device: &'a Id,
    /// backend buffer table.
    pub(crate) buffers: &'a HandleTable<BufferKind, MetalBuffer>,
    /// pipeline cache.
    pub(crate) pipelines: &'a mut PipelineCache,
}

/// 검사를 마친 encoder 호출. native 포인터는 table·cache가 소유합니다(+0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Op {
    /// `setRenderPipelineState:`.
    Pipeline(RawId),
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

#[derive(Debug, Default)]
struct DrawState {
    pipeline: Option<PipelineKey>,
    vertex: Option<(BufferHandle, u64)>,
    index: Option<(BufferHandle, u64)>,
    uniforms: bool,
    scissor_empty: bool,
}

/// bind한 vertex buffer에 들어가는 vertex 수.
fn vertex_capacity(
    ctx: &CommandContext<'_>,
    state: &DrawState,
    key: &PipelineKey,
) -> Result<u64, BackendError> {
    let (handle, offset) = state.vertex.ok_or(invalid("vertex buffer가 없습니다"))?;
    let buffer = ctx.buffers.get(handle)?;
    let stride = vertex_stride(key.program).ok_or(BackendError::new(
        BackendErrorKind::Unsupported,
        "vertex layout이 없는 shader program",
    ))?;
    Ok(buffer.desc.size.saturating_sub(offset) / u64::from(stride))
}

/// command 목록을 검사하고 encoder 호출로 바꿉니다.
pub(crate) fn plan_commands(
    ctx: &mut CommandContext<'_>,
    pass: &PassFormat,
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
                let pipeline = ctx.pipelines.get(ctx.device, &key)?;
                state.pipeline = Some(key);
                ops.push(Op::Pipeline(pipeline));
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
                ops.push(Op::StencilReference(u32::from(reference)));
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
                state.uniforms = true;
                ops.push(Op::Uniforms {
                    buffer: native.buffer.as_raw(),
                    offset: offset as NSUInteger,
                });
            }
            DrawCommand::BindTexture { .. } => {
                return Err(BackendError::new(
                    BackendErrorKind::Unsupported,
                    "texture binding은 아직 지원하지 않습니다",
                ));
            }
            DrawCommand::Draw {
                vertex_start,
                vertex_count,
                instance_count,
            } => {
                let key = state
                    .pipeline
                    .ok_or(invalid("draw 전에 pipeline을 bind해야 합니다"))?;
                if !state.uniforms {
                    return Err(invalid("draw 전에 uniform을 bind해야 합니다"));
                }
                let capacity = vertex_capacity(ctx, &state, &key)?;
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
                let key = state
                    .pipeline
                    .ok_or(invalid("draw 전에 pipeline을 bind해야 합니다"))?;
                if !state.uniforms {
                    return Err(invalid("draw 전에 uniform을 bind해야 합니다"));
                }
                let capacity = vertex_capacity(ctx, &state, &key)?;
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
    // NSUInteger, NSUInteger), setRenderPipelineState:(id), setScissorRect:(MTLScissorRect),
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

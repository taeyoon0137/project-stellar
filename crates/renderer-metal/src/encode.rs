//! RenderGraph pass → `MTLRenderCommandEncoder` encoding.
//!
//! encoding은 두 단계입니다.
//!
//! 1. **계획**([`plan_pass`]): pass attachment를 실제 texture로 해석하고 format·크기·sample 수를
//!    검사하며, draw command를 검사한 encoder 호출로 바꿉니다([`crate::commands`]). pipeline 생성도
//!    이 단계에서 끝납니다. 실패해도 command buffer에는 아무것도 기록되지 않습니다.
//! 2. **기록**([`encode_pass`]): 계획만 보고 Metal 호출을 합니다. encoder를 연 뒤에는 실패 경로가 없어
//!    `endEncoding` 없이 encoder가 해제되는 일(Metal assertion)이 생기지 않습니다.

use stellar_renderer_core::backend::{
    BackendError, BackendErrorKind, FrameEncodeRequest, ImportTarget, SurfaceSize,
};
use stellar_renderer_core::gpu_handle::{HandleTable, TextureKind};
use stellar_renderer_core::render_graph::{
    AttachmentSize, ClearColor, ImportKind, LoadOp, PassId, PassKind, ResourceId, ResourceLifetime,
    SampleCount, StoreOp, TextureFormat,
};

use crate::commands::{plan_commands, record, CommandContext, Op, PassFormat};
use crate::objc::{class, sel, send, Id, NSUInteger, RawId};
use crate::surface::Drawable;
use crate::sys::{load_action, store_action, MTLClearColor};
use crate::texture::MetalTexture;

/// attachment가 가리키는 texture(frame encode 동안 table·drawable이 살아 있게 유지).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Target {
    /// `id<MTLTexture>`(+0, 소유자는 texture table 또는 열린 frame의 drawable).
    pub(crate) texture: RawId,
    /// 너비.
    pub(crate) width: u32,
    /// 높이.
    pub(crate) height: u32,
    /// format.
    pub(crate) format: TextureFormat,
    /// sample 수.
    pub(crate) samples: SampleCount,
}

/// frame의 attachment 해석 문맥.
pub(crate) struct FrameTargets<'a> {
    /// encode 요청.
    pub(crate) request: &'a FrameEncodeRequest<'a>,
    /// 이 frame이 얻은 drawable(surface import가 있을 때).
    pub(crate) drawable: Option<&'a Drawable>,
    /// 현재 surface 크기.
    pub(crate) surface_size: SurfaceSize,
    /// surface format.
    pub(crate) surface_format: TextureFormat,
    /// backend texture table.
    pub(crate) textures: &'a HandleTable<TextureKind, MetalTexture>,
}

fn invalid(detail: &'static str) -> BackendError {
    BackendError::new(BackendErrorKind::InvalidUsage, detail)
}

impl FrameTargets<'_> {
    /// graph resource를 texture로 해석합니다.
    pub(crate) fn resolve(&self, id: ResourceId) -> Result<Target, BackendError> {
        let resource = self
            .request
            .graph
            .resource(id)
            .ok_or(invalid("없는 graph resource"))?;
        let desc = resource.desc;
        let (width, height) = match desc.size {
            AttachmentSize::Surface => (self.surface_size.width, self.surface_size.height),
            AttachmentSize::Fixed { width, height } => (width, height),
        };
        match resource.lifetime {
            ResourceLifetime::Imported(ImportKind::Surface) => {
                let drawable = self.drawable.ok_or(invalid("surface drawable 없음"))?;
                if desc.format != self.surface_format || desc.samples != SampleCount::X1 {
                    return Err(invalid("surface resource format이 surface 설정과 다릅니다"));
                }
                Ok(Target {
                    texture: drawable.texture().as_raw(),
                    width: self.surface_size.width,
                    height: self.surface_size.height,
                    format: desc.format,
                    samples: SampleCount::X1,
                })
            }
            ResourceLifetime::Imported(ImportKind::External) => {
                let binding = self
                    .request
                    .imports
                    .iter()
                    .find(|binding| binding.resource == id)
                    .ok_or(invalid("imported resource binding 없음"))?;
                let ImportTarget::Texture(handle) = binding.target else {
                    return Err(invalid("external resource에 surface binding"));
                };
                let texture = self.textures.get(handle)?;
                if texture.desc.format != desc.format || texture.desc.samples != desc.samples {
                    return Err(invalid("external texture format이 graph와 다릅니다"));
                }
                if texture.desc.width != width || texture.desc.height != height {
                    return Err(invalid("external texture 크기가 graph와 다릅니다"));
                }
                Ok(Target {
                    texture: texture.texture.as_raw(),
                    width,
                    height,
                    format: desc.format,
                    samples: desc.samples,
                })
            }
            ResourceLifetime::Transient => Err(BackendError::new(
                BackendErrorKind::Unsupported,
                "transient graph resource는 아직 지원하지 않습니다",
            )),
        }
    }
}

/// color attachment 계획.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ColorPlan {
    target: Target,
    load: NSUInteger,
    store: NSUInteger,
    clear: MTLClearColor,
}

/// pass 하나의 encoding 계획.
#[derive(Debug, Clone)]
pub(crate) struct PassPlan {
    colors: Vec<ColorPlan>,
    ops: Vec<Op>,
    width: u32,
    height: u32,
}

const fn load(op: LoadOp) -> NSUInteger {
    match op {
        LoadOp::Load => load_action::LOAD,
        LoadOp::Clear => load_action::CLEAR,
        LoadOp::DontCare => load_action::DONT_CARE,
    }
}

const fn store(op: StoreOp) -> NSUInteger {
    match op {
        StoreOp::Store => store_action::STORE,
        StoreOp::Discard => store_action::DONT_CARE,
    }
}

fn clear_color(clear: ClearColor) -> MTLClearColor {
    MTLClearColor {
        red: f64::from(clear.r.get()),
        green: f64::from(clear.g.get()),
        blue: f64::from(clear.b.get()),
        alpha: f64::from(clear.a.get()),
    }
}

/// pass attachment와 draw command를 해석하고 검사합니다.
pub(crate) fn plan_pass(
    targets: &FrameTargets<'_>,
    commands: &mut CommandContext<'_>,
    pass: PassId,
) -> Result<PassPlan, BackendError> {
    let graph_pass = targets
        .request
        .graph
        .pass(pass)
        .ok_or(invalid("없는 graph pass"))?;
    if graph_pass.kind != PassKind::Raster {
        return Err(BackendError::new(
            BackendErrorKind::Unsupported,
            "Metal backend는 아직 raster pass만 encode합니다",
        ));
    }
    if graph_pass.depth_stencil.is_some() {
        return Err(BackendError::new(
            BackendErrorKind::Unsupported,
            "depth·stencil attachment는 아직 지원하지 않습니다",
        ));
    }
    let mut colors = Vec::with_capacity(graph_pass.color.len());
    let mut extent: Option<(u32, u32)> = None;
    for attachment in &graph_pass.color {
        let target = targets.resolve(attachment.target.resource)?;
        if target.format.is_depth_stencil() {
            return Err(invalid("color attachment에 depth format"));
        }
        if target.width == 0 || target.height == 0 {
            return Err(invalid("0 크기 attachment"));
        }
        match extent {
            None => extent = Some((target.width, target.height)),
            Some(size) if size != (target.width, target.height) => {
                return Err(invalid("pass attachment 크기가 서로 다릅니다"));
            }
            Some(_) => {}
        }
        colors.push(ColorPlan {
            target,
            load: load(attachment.load),
            store: store(attachment.store),
            clear: clear_color(attachment.clear),
        });
    }
    let (width, height) = extent.ok_or(invalid("attachment 없는 raster pass"))?;
    let first = colors[0].target;
    let format = PassFormat {
        color_format: first.format,
        samples: first.samples,
        color_count: colors.len(),
        width,
        height,
    };
    let ops = match targets.request.commands_for(pass) {
        Some(list) => plan_commands(commands, &format, &list.commands)?,
        None => Vec::new(),
    };
    Ok(PassPlan {
        colors,
        ops,
        width,
        height,
    })
}

/// 계획한 pass를 `command_buffer`에 기록합니다.
///
/// 계획의 texture는 호출 동안 살아 있어야 합니다(table·열린 frame 소유). render pass descriptor 등
/// 임시 객체는 호출자의 autorelease pool에 쌓입니다.
pub(crate) fn encode_pass(command_buffer: &Id, plan: &PassPlan) -> Result<(), BackendError> {
    let descriptor_class = class!("MTLRenderPassDescriptor").ok_or(BackendError::new(
        BackendErrorKind::Unsupported,
        "MTLRenderPassDescriptor class가 없습니다",
    ))?;
    // SAFETY: `+renderPassDescriptor`는 () -> MTLRenderPassDescriptor*(+0)이고, `-colorAttachments`는
    // () -> array, `-objectAtIndexedSubscript:`는 (NSUInteger) -> attachment descriptor(index는
    // MAX_COLOR_ATTACHMENTS 미만, graph 검증)입니다. attachment setter: setTexture:(id),
    // setLoadAction:/setStoreAction:(NSUInteger), setClearColor:(MTLClearColor). 계획의 texture는
    // 살아 있습니다(함수 문서).
    unsafe {
        let descriptor: RawId = send(descriptor_class.as_raw(), sel!("renderPassDescriptor"), ());
        if descriptor.is_null() {
            return Err(BackendError::new(
                BackendErrorKind::OutOfHostMemory,
                "render pass descriptor를 만들 수 없습니다",
            ));
        }
        let attachments: RawId = send(descriptor, sel!("colorAttachments"), ());
        for (index, color) in plan.colors.iter().enumerate() {
            let attachment: RawId = send(
                attachments,
                sel!("objectAtIndexedSubscript:"),
                (index as NSUInteger,),
            );
            let _: () = send(attachment, sel!("setTexture:"), (color.target.texture,));
            let _: () = send(attachment, sel!("setLoadAction:"), (color.load,));
            let _: () = send(attachment, sel!("setStoreAction:"), (color.store,));
            let _: () = send(attachment, sel!("setClearColor:"), (color.clear,));
        }
        let encoder: RawId =
            command_buffer.send(sel!("renderCommandEncoderWithDescriptor:"), (descriptor,));
        if encoder.is_null() {
            return Err(BackendError::new(
                BackendErrorKind::Internal,
                "render command encoder를 만들 수 없습니다",
            ));
        }
        record(encoder, &plan.ops, plan.width, plan.height);
        let _: () = send(encoder, sel!("endEncoding"), ());
    }
    Ok(())
}

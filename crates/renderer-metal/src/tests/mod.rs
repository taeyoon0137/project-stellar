//! 실제 GPU를 쓰는 Metal backend 테스트.
//!
//! 모든 테스트는 offscreen render-to-texture 또는 창에 붙지 않은 CAMetalLayer drawable을 그린 뒤
//! pixel을 CPU로 읽어 확인합니다. Metal device가 없는 환경(가상 macOS 등)에서는 테스트가 그 사실을
//! 출력하고 검증 없이 끝납니다([`metal_available`]).
//!
//! crate 의존 정책(`tools/xtask` crate graph)상 이 crate는 `stellar-renderer-core`에만 의존하므로
//! scene 타입(`ColorSpace`·`FiniteF32`)을 이름으로 쓰지 못합니다. 테스트는 renderer-core 공개 값에서
//! 그 값을 얻습니다([`device_config`], [`unit_clear`]).

use stellar_renderer_core::backend::{
    DeviceConfig, FrameEncodeRequest, FrameId, GpuTextureDesc, ImportBinding, ImportTarget,
    SurfaceConfig, SurfaceSize, TextureHandle, TextureUsage,
};
use stellar_renderer_core::placeholder::{placeholder_texture, PlaceholderKind};
use stellar_renderer_core::render_graph::{
    AttachmentSize, ClearColor, CompiledRenderGraph, ImportKind, LoadOp, PassKind,
    RenderGraphBuilder, SampleCount, StoreOp, TextureDesc, TextureFormat,
};

use crate::objc::{sel, Id};
use crate::sys::MTLCreateSystemDefaultDevice;

mod smoke;
mod surface;

/// 이 machine에 Metal device가 있으면 참입니다. 없으면 이유를 출력합니다.
pub(crate) fn metal_available() -> bool {
    // SAFETY: Create 규칙(+1) 또는 nil이며 Id가 해제합니다.
    let available = unsafe { Id::from_owned(MTLCreateSystemDefaultDevice()) }.is_some();
    if !available {
        eprintln!("Metal device 없음: GPU 테스트를 검증 없이 건너뜁니다");
    }
    available
}

/// 각 채널을 0 또는 1로 둔 clear 색.
///
/// `FiniteF32`를 이름으로 쓸 수 없어 0은 [`ClearColor::TRANSPARENT`], 1은 graph builder의 depth
/// clear 값(1.0)에서 얻습니다.
pub(crate) fn unit_clear(rgba: [bool; 4]) -> ClearColor {
    let zero = ClearColor::TRANSPARENT.r;
    let mut builder = RenderGraphBuilder::new();
    let depth = builder.transient(
        "depth",
        TextureDesc::surface(TextureFormat::Depth32FloatStencil8),
    );
    let pass = builder.pass("depth", PassKind::Raster);
    builder
        .depth_stencil(pass, depth, LoadOp::Clear, StoreOp::Discard)
        .expect("depth attachment");
    let one = builder.finish().passes[0]
        .depth_stencil
        .expect("depth attachment")
        .clear_depth;
    let pick = |on: bool| if on { one } else { zero };
    ClearColor {
        r: pick(rgba[0]),
        g: pick(rgba[1]),
        b: pick(rgba[2]),
        a: pick(rgba[3]),
    }
}

/// surface 설정으로 device config를 만듭니다.
pub(crate) fn device_config(width: u32, height: u32, format: TextureFormat) -> DeviceConfig {
    DeviceConfig {
        surface: SurfaceConfig {
            size: SurfaceSize { width, height },
            format,
            color_space: placeholder_texture(PlaceholderKind::Missing).color_space(),
        },
        frames_in_flight: 3,
        timestamps: false,
    }
}

/// offscreen render target texture descriptor.
pub(crate) fn target_desc(width: u32, height: u32, format: TextureFormat) -> GpuTextureDesc {
    GpuTextureDesc {
        width,
        height,
        format,
        mip_levels: 1,
        samples: SampleCount::X1,
        usage: TextureUsage {
            sampled: true,
            render_target: true,
        },
    }
}

/// external texture 하나를 `clear`로 지우는 graph.
pub(crate) fn external_clear_graph(
    width: u32,
    height: u32,
    format: TextureFormat,
    clear: ClearColor,
) -> CompiledRenderGraph {
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import(
        "target",
        TextureDesc {
            format,
            size: AttachmentSize::Fixed { width, height },
            samples: SampleCount::X1,
        },
        ImportKind::External,
    );
    let pass = builder.pass("clear", PassKind::Raster);
    let written = builder
        .color(pass, target, LoadOp::Clear, clear, StoreOp::Store)
        .expect("color attachment");
    builder.set_output(written);
    builder.finish().compile().expect("graph compile")
}

/// surface drawable을 `clear`로 지우는 graph.
pub(crate) fn surface_clear_graph(format: TextureFormat, clear: ClearColor) -> CompiledRenderGraph {
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import("surface", TextureDesc::surface(format), ImportKind::Surface);
    let pass = builder.pass("clear", PassKind::Raster);
    let written = builder
        .color(pass, target, LoadOp::Clear, clear, StoreOp::Store)
        .expect("color attachment");
    builder.set_output(written);
    builder.finish().compile().expect("graph compile")
}

/// graph의 첫 imported resource를 `texture`에 묶는 binding.
pub(crate) fn bind_first_import(
    graph: &CompiledRenderGraph,
    texture: TextureHandle,
) -> Vec<ImportBinding> {
    vec![ImportBinding {
        resource: graph.output().expect("output").resource,
        target: ImportTarget::Texture(texture),
    }]
}

/// graph output(surface)을 drawable에 묶는 binding.
pub(crate) fn bind_surface(graph: &CompiledRenderGraph) -> Vec<ImportBinding> {
    vec![ImportBinding {
        resource: graph.output().expect("output").resource,
        target: ImportTarget::Surface,
    }]
}

/// command 없는 encode 요청.
pub(crate) fn request<'a>(
    frame: u64,
    graph: &'a CompiledRenderGraph,
    imports: &'a [ImportBinding],
) -> FrameEncodeRequest<'a> {
    FrameEncodeRequest {
        frame: FrameId(frame),
        graph,
        passes: &[],
        imports,
    }
}

/// shared buffer의 앞 `length` byte.
pub(crate) fn buffer_bytes(buffer: &Id, length: usize) -> Vec<u8> {
    // SAFETY: `-contents`는 () -> void*이고 shared buffer는 length byte 이상입니다(capture가 할당).
    unsafe {
        let contents: *const u8 = buffer.send(sel!("contents"), ());
        std::slice::from_raw_parts(contents, length).to_vec()
    }
}

/// 모든 pixel이 `pixel`이면 참입니다.
pub(crate) fn all_pixels(bytes: &[u8], pixel: [u8; 4]) -> bool {
    !bytes.is_empty() && bytes.chunks_exact(4).all(|chunk| chunk == pixel)
}

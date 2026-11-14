//! `PipelineKey` → `MTLRenderPipelineState` 생성과 cache.
//!
//! 내장 shader library는 처음 pipeline이 필요할 때 한 번 compile합니다(clear만 하는 frame은 compile
//! 비용이 없음). pipeline은 key마다 한 번 만들어 device 수명 동안 보관합니다.

use std::collections::HashMap;
use std::ffi::c_void;

use stellar_renderer_core::backend::{
    BackendError, BackendErrorKind, BlendState, DepthMode, PipelineKey, StencilMode,
};

use crate::format::mtl_pixel_format;
use crate::objc::{class, ns_string, sel, send, Id, NSUInteger, RawId};
use crate::shaders::{entry_points, BUILTIN_SOURCE};
use crate::texture::sample_count;

/// `MTLBlendFactorOne`.
const BLEND_ONE: NSUInteger = 1;
/// `MTLBlendFactorOneMinusSourceAlpha`.
const BLEND_ONE_MINUS_SOURCE_ALPHA: NSUInteger = 5;

fn pipeline_error(detail: &'static str) -> BackendError {
    BackendError::new(BackendErrorKind::PipelineCreation, detail)
}

/// pipeline state cache.
#[derive(Debug, Default)]
pub(crate) struct PipelineCache {
    library: Option<Id>,
    states: HashMap<PipelineKey, Id>,
}

impl PipelineCache {
    /// 빈 cache.
    pub(crate) fn new() -> PipelineCache {
        PipelineCache::default()
    }

    /// 만든 pipeline 수.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.states.len()
    }

    fn library(&mut self, device: &Id) -> Result<&Id, BackendError> {
        if self.library.is_none() {
            self.library = Some(compile_library(device, BUILTIN_SOURCE)?);
        }
        self.library
            .as_ref()
            .ok_or(pipeline_error("shader library 없음"))
    }

    /// key의 pipeline state(+0, cache 소유). 없으면 만듭니다.
    pub(crate) fn get(&mut self, device: &Id, key: &PipelineKey) -> Result<RawId, BackendError> {
        if let Some(state) = self.states.get(key) {
            return Ok(state.as_raw());
        }
        let (vertex, fragment) = entry_points(key.program).ok_or(BackendError::new(
            BackendErrorKind::Unsupported,
            "Metal backend가 아직 구현하지 않은 shader program",
        ))?;
        if key.depth != DepthMode::Disabled || key.stencil != StencilMode::Disabled {
            return Err(BackendError::new(
                BackendErrorKind::Unsupported,
                "depth·stencil pipeline은 아직 지원하지 않습니다",
            ));
        }
        if key.color_format.is_depth_stencil() {
            return Err(BackendError::new(
                BackendErrorKind::InvalidUsage,
                "pipeline color format에 depth format",
            ));
        }
        let library = self.library(device)?;
        let vertex = function(library, vertex)?;
        let fragment = function(library, fragment)?;
        let state = create_state(device, key, &vertex, &fragment)?;
        let raw = state.as_raw();
        self.states.insert(*key, state);
        Ok(raw)
    }
}

/// MSL 소스를 compile합니다.
fn compile_library(device: &Id, source: &str) -> Result<Id, BackendError> {
    let source = ns_string(source).ok_or(BackendError::new(
        BackendErrorKind::OutOfHostMemory,
        "shader 소스 문자열을 만들 수 없습니다",
    ))?;
    let mut error: RawId = std::ptr::null_mut();
    // SAFETY: `-newLibraryWithSource:options:error:`는 (NSString*, MTLCompileOptions*, NSError**) ->
    // id<MTLLibrary>(+1, 실패 시 nil)입니다. options nil은 기본값, error는 autoreleased(+0)입니다.
    let library: RawId = unsafe {
        device.send(
            sel!("newLibraryWithSource:options:error:"),
            (
                source.as_raw(),
                std::ptr::null_mut::<c_void>(),
                &mut error as *mut RawId,
            ),
        )
    };
    // SAFETY: library는 +1 또는 nil입니다.
    unsafe { Id::from_owned(library) }.ok_or(pipeline_error("MSL shader compile 실패"))
}

/// library에서 함수를 찾습니다.
fn function(library: &Id, name: &str) -> Result<Id, BackendError> {
    let name = ns_string(name).ok_or(BackendError::new(
        BackendErrorKind::OutOfHostMemory,
        "함수 이름 문자열을 만들 수 없습니다",
    ))?;
    // SAFETY: `-newFunctionWithName:`은 (NSString*) -> id<MTLFunction>(+1, 없으면 nil)입니다.
    unsafe { Id::from_owned(library.send(sel!("newFunctionWithName:"), (name.as_raw(),))) }
        .ok_or(pipeline_error("shader entry point가 library에 없습니다"))
}

/// pipeline descriptor를 채우고 state를 만듭니다.
fn create_state(
    device: &Id,
    key: &PipelineKey,
    vertex: &Id,
    fragment: &Id,
) -> Result<Id, BackendError> {
    let class = class!("MTLRenderPipelineDescriptor").ok_or(BackendError::new(
        BackendErrorKind::Unsupported,
        "MTLRenderPipelineDescriptor class가 없습니다",
    ))?;
    // SAFETY: `+new`는 () -> id(+1)입니다. descriptor setter: setVertexFunction:/setFragmentFunction:
    // (id), setRasterSampleCount:(NSUInteger), colorAttachments → objectAtIndexedSubscript:(0) →
    // setPixelFormat:/setBlendingEnabled:(BOOL)/set*BlendFactor:(NSUInteger). error는 +0입니다.
    unsafe {
        let descriptor =
            Id::from_owned(send(class.as_raw(), sel!("new"), ())).ok_or(BackendError::new(
                BackendErrorKind::OutOfHostMemory,
                "pipeline descriptor를 만들 수 없습니다",
            ))?;
        let _: () = descriptor.send(sel!("setVertexFunction:"), (vertex.as_raw(),));
        let _: () = descriptor.send(sel!("setFragmentFunction:"), (fragment.as_raw(),));
        let _: () = descriptor.send(sel!("setRasterSampleCount:"), (sample_count(key.samples),));
        let attachments: RawId = descriptor.send(sel!("colorAttachments"), ());
        let color: RawId = send(
            attachments,
            sel!("objectAtIndexedSubscript:"),
            (0 as NSUInteger,),
        );
        let _: () = send(
            color,
            sel!("setPixelFormat:"),
            (mtl_pixel_format(key.color_format),),
        );
        match key.blend {
            BlendState::Replace => {
                let _: () = send(color, sel!("setBlendingEnabled:"), (false,));
            }
            BlendState::PremultipliedOver => {
                let _: () = send(color, sel!("setBlendingEnabled:"), (true,));
                let _: () = send(color, sel!("setSourceRGBBlendFactor:"), (BLEND_ONE,));
                let _: () = send(color, sel!("setSourceAlphaBlendFactor:"), (BLEND_ONE,));
                let _: () = send(
                    color,
                    sel!("setDestinationRGBBlendFactor:"),
                    (BLEND_ONE_MINUS_SOURCE_ALPHA,),
                );
                let _: () = send(
                    color,
                    sel!("setDestinationAlphaBlendFactor:"),
                    (BLEND_ONE_MINUS_SOURCE_ALPHA,),
                );
            }
        }
        let mut error: RawId = std::ptr::null_mut();
        let state: RawId = device.send(
            sel!("newRenderPipelineStateWithDescriptor:error:"),
            (descriptor.as_raw(), &mut error as *mut RawId),
        );
        Id::from_owned(state).ok_or(pipeline_error("render pipeline state 생성 실패"))
    }
}

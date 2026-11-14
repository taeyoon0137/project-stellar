//! shader library cache와 pipeline·depth-stencil·sampler state cache (XI.8.4).
//!
//! - **shader library cache**([`ShaderLibraryCache`]): MSL 소스 digest(FNV-1a 64 + 소스 비교)마다
//!   compile 결과를 한 번만 만듭니다. **실패도 기억**해 깨진 소스를 frame마다 다시 compile하지
//!   않습니다(compile은 수십 ms라 frame 예산을 넘음).
//! - **pipeline cache**: (`PipelineKey`, pass depth attachment format)마다 `MTLRenderPipelineState`를 한
//!   번 만듭니다. pass에 depth·stencil attachment가 있으면 그 안의 모든 pipeline이 format을 선언해야
//!   하므로(Metal 규칙) key에 포함합니다.
//! - **depth-stencil·sampler cache**: `(DepthMode, StencilMode)`와 `SamplerDesc`마다 state 하나.
//! - **구조화 오류**: compile·entry point·pipeline 생성 실패는 [`ShaderError`](단계 + Metal compiler
//!   log)로 남기고 `BackendErrorKind::PipelineCreation`을 돌려줍니다. log는 내장 shader 소스의 진단이며
//!   사용자 내용은 들어 있지 않습니다(정적 detail만 오류 값에 넣는 규칙과 별도 보관).
//!
//! state는 device 수명 동안 보관합니다(재 init 시 cache 전체를 버림).

use std::collections::HashMap;
use std::ffi::c_void;
use std::fmt;

use stellar_renderer_core::backend::{
    BackendError, BackendErrorKind, BlendState, DepthMode, PipelineKey, ShaderProgram, StencilMode,
};
use stellar_renderer_core::placeholder::{SamplerDesc, TextureFilter, TextureWrap};
use stellar_renderer_core::render_graph::TextureFormat;

use crate::format::mtl_pixel_format;
use crate::objc::{class, ns_string, rust_string, sel, send, Id, NSUInteger, RawId};
use crate::shaders::{program_layout, ProgramLayout, BUILTIN_SOURCE};
use crate::texture::sample_count;

/// `MTLBlendFactorOne`.
const BLEND_ONE: NSUInteger = 1;
/// `MTLBlendFactorOneMinusSourceAlpha`.
const BLEND_ONE_MINUS_SOURCE_ALPHA: NSUInteger = 5;
/// `MTLColorWriteMaskNone`.
const WRITE_MASK_NONE: NSUInteger = 0;
/// `MTLCompareFunctionLess`.
const COMPARE_LESS: NSUInteger = 1;
/// `MTLCompareFunctionEqual`.
const COMPARE_EQUAL: NSUInteger = 2;
/// `MTLCompareFunctionAlways`.
const COMPARE_ALWAYS: NSUInteger = 7;
/// `MTLStencilOperationKeep`.
const STENCIL_KEEP: NSUInteger = 0;
/// `MTLStencilOperationIncrementClamp`.
const STENCIL_INCREMENT_CLAMP: NSUInteger = 3;
/// `MTLStencilOperationDecrementClamp`.
const STENCIL_DECREMENT_CLAMP: NSUInteger = 4;
/// `MTLSamplerMinMagFilterNearest`·`Linear`.
const FILTER_NEAREST: NSUInteger = 0;
const FILTER_LINEAR: NSUInteger = 1;
/// `MTLSamplerMipFilterNotMipmapped`·`Linear`.
const MIP_NONE: NSUInteger = 0;
const MIP_LINEAR: NSUInteger = 2;
/// `MTLSamplerAddressModeClampToEdge`·`Repeat`.
const ADDRESS_CLAMP: NSUInteger = 0;
const ADDRESS_REPEAT: NSUInteger = 2;

/// 보관하는 compiler log 최대 byte.
const MAX_LOG_BYTES: usize = 4096;

/// shader 오류 단계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShaderErrorStage {
    /// MSL 소스 compile 실패.
    Compile,
    /// library에 entry point 함수가 없음.
    MissingEntryPoint,
    /// render pipeline state 생성 실패.
    PipelineState,
}

/// 구조화 shader 오류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShaderError {
    /// 실패 단계.
    pub stage: ShaderErrorStage,
    /// 대상 program(library compile이면 `None`).
    pub program: Option<ShaderProgram>,
    /// Metal compiler·runtime log(최대 4096 byte, 없으면 빈 문자열).
    pub log: String,
}

impl fmt::Display for ShaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "shader {:?} 실패", self.stage)?;
        if let Some(program) = self.program {
            write!(f, " ({program:?})")?;
        }
        if !self.log.is_empty() {
            write!(f, ": {}", self.log)?;
        }
        Ok(())
    }
}

impl std::error::Error for ShaderError {}

impl ShaderError {
    /// 공통 계층 오류(정적 detail).
    pub(crate) fn backend_error(&self) -> BackendError {
        let detail = match self.stage {
            ShaderErrorStage::Compile => "MSL shader compile 실패",
            ShaderErrorStage::MissingEntryPoint => "shader entry point가 library에 없습니다",
            ShaderErrorStage::PipelineState => "render pipeline state 생성 실패",
        };
        BackendError::new(BackendErrorKind::PipelineCreation, detail)
    }
}

/// `NSError*`의 설명 문자열.
fn error_log(error: RawId) -> String {
    if error.is_null() {
        return String::new();
    }
    // SAFETY: error는 Metal이 out-parameter로 돌려준 NSError(+0)이고 `-localizedDescription`은
    // () -> NSString*입니다.
    let description: RawId = unsafe { send(error, sel!("localizedDescription"), ()) };
    rust_string(description, MAX_LOG_BYTES).unwrap_or_default()
}

/// FNV-1a 64.
fn digest(source: &str) -> u64 {
    source.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[derive(Debug)]
struct LibraryEntry {
    source: String,
    result: Result<Id, ShaderError>,
}

/// MSL 소스 → `MTLLibrary` cache(실패 포함).
#[derive(Debug, Default)]
pub(crate) struct ShaderLibraryCache {
    entries: HashMap<u64, Vec<LibraryEntry>>,
    compiles: u64,
}

impl ShaderLibraryCache {
    /// 지금까지 실제로 compile한 횟수.
    pub(crate) fn compiles(&self) -> u64 {
        self.compiles
    }

    /// 소스의 library. 처음이면 compile하고 결과(성공·실패)를 보관합니다.
    pub(crate) fn library(&mut self, device: &Id, source: &str) -> Result<&Id, ShaderError> {
        let bucket = self.entries.entry(digest(source)).or_default();
        let index = match bucket.iter().position(|entry| entry.source == source) {
            Some(index) => index,
            None => {
                self.compiles += 1;
                bucket.push(LibraryEntry {
                    source: source.to_owned(),
                    result: compile_library(device, source),
                });
                bucket.len() - 1
            }
        };
        bucket[index].result.as_ref().map_err(Clone::clone)
    }
}

/// MSL 소스를 compile합니다.
fn compile_library(device: &Id, source: &str) -> Result<Id, ShaderError> {
    let compile_error = |log| ShaderError {
        stage: ShaderErrorStage::Compile,
        program: None,
        log,
    };
    let text = ns_string(source).ok_or(compile_error(String::new()))?;
    let mut error: RawId = std::ptr::null_mut();
    // SAFETY: `-newLibraryWithSource:options:error:`는 (NSString*, MTLCompileOptions*, NSError**) ->
    // id<MTLLibrary>(+1, 실패 시 nil)입니다. options nil은 기본값, error는 autoreleased(+0)입니다.
    let library: RawId = unsafe {
        device.send(
            sel!("newLibraryWithSource:options:error:"),
            (
                text.as_raw(),
                std::ptr::null_mut::<c_void>(),
                &mut error as *mut RawId,
            ),
        )
    };
    // SAFETY: library는 +1 또는 nil입니다.
    unsafe { Id::from_owned(library) }.ok_or_else(|| compile_error(error_log(error)))
}

/// library에서 함수를 찾습니다.
fn function(library: &Id, name: &str, program: ShaderProgram) -> Result<Id, ShaderError> {
    let missing = || ShaderError {
        stage: ShaderErrorStage::MissingEntryPoint,
        program: Some(program),
        log: name.to_owned(),
    };
    let text = ns_string(name).ok_or_else(missing)?;
    // SAFETY: `-newFunctionWithName:`은 (NSString*) -> id<MTLFunction>(+1, 없으면 nil)입니다.
    unsafe { Id::from_owned(library.send(sel!("newFunctionWithName:"), (text.as_raw(),))) }
        .ok_or_else(missing)
}

/// pipeline cache key: 공통 key + pass depth·stencil attachment format.
type StateKey = (PipelineKey, Option<TextureFormat>);

/// pipeline·depth-stencil·sampler state cache.
#[derive(Debug)]
pub(crate) struct PipelineCache {
    source: &'static str,
    libraries: ShaderLibraryCache,
    states: HashMap<StateKey, Id>,
    depth_states: HashMap<(DepthMode, StencilMode), Id>,
    samplers: HashMap<SamplerDesc, Id>,
    last_error: Option<ShaderError>,
}

impl Default for PipelineCache {
    fn default() -> PipelineCache {
        PipelineCache::new()
    }
}

impl PipelineCache {
    /// 내장 shader를 쓰는 빈 cache.
    pub(crate) fn new() -> PipelineCache {
        PipelineCache::with_source(BUILTIN_SOURCE)
    }

    /// 지정 MSL 소스를 쓰는 빈 cache(테스트에서 compile 실패 경로를 재현).
    pub(crate) fn with_source(source: &'static str) -> PipelineCache {
        PipelineCache {
            source,
            libraries: ShaderLibraryCache::default(),
            states: HashMap::new(),
            depth_states: HashMap::new(),
            samplers: HashMap::new(),
            last_error: None,
        }
    }

    /// 만든 pipeline 수.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.states.len()
    }

    /// 실제 library compile 횟수.
    pub(crate) fn library_compiles(&self) -> u64 {
        self.libraries.compiles()
    }

    /// 마지막 shader 오류.
    pub(crate) fn last_error(&self) -> Option<&ShaderError> {
        self.last_error.as_ref()
    }

    fn fail(&mut self, error: ShaderError) -> BackendError {
        let backend = error.backend_error();
        self.last_error = Some(error);
        backend
    }

    /// key의 pipeline state(+0, cache 소유). 없으면 만듭니다.
    pub(crate) fn pipeline(
        &mut self,
        device: &Id,
        key: &PipelineKey,
        depth_format: Option<TextureFormat>,
    ) -> Result<RawId, BackendError> {
        if let Some(state) = self.states.get(&(*key, depth_format)) {
            return Ok(state.as_raw());
        }
        let layout = program_layout(key.program).ok_or(BackendError::new(
            BackendErrorKind::Unsupported,
            "Metal backend가 아직 구현하지 않은 shader program",
        ))?;
        if key.color_format.is_depth_stencil() {
            return Err(BackendError::new(
                BackendErrorKind::InvalidUsage,
                "pipeline color format에 depth format",
            ));
        }
        if depth_format.is_none()
            && (key.depth != DepthMode::Disabled || key.stencil != StencilMode::Disabled)
        {
            return Err(BackendError::new(
                BackendErrorKind::InvalidUsage,
                "depth·stencil pipeline은 depth·stencil attachment가 있는 pass에서만 씁니다",
            ));
        }
        let library = match self.libraries.library(device, self.source) {
            Ok(library) => library.clone(),
            Err(error) => return Err(self.fail(error)),
        };
        let functions = function(&library, layout.vertex, key.program).and_then(|vertex| {
            function(&library, layout.fragment, key.program).map(|fragment| (vertex, fragment))
        });
        let (vertex, fragment) = match functions {
            Ok(functions) => functions,
            Err(error) => return Err(self.fail(error)),
        };
        let state = match create_state(device, key, depth_format, &layout, &vertex, &fragment) {
            Ok(state) => state,
            Err(error) => return Err(self.fail(error)),
        };
        let raw = state.as_raw();
        self.states.insert((*key, depth_format), state);
        Ok(raw)
    }

    /// depth·stencil state(+0, cache 소유).
    pub(crate) fn depth_stencil(
        &mut self,
        device: &Id,
        depth: DepthMode,
        stencil: StencilMode,
    ) -> Result<RawId, BackendError> {
        if let Some(state) = self.depth_states.get(&(depth, stencil)) {
            return Ok(state.as_raw());
        }
        let state = create_depth_stencil(device, depth, stencil)?;
        let raw = state.as_raw();
        self.depth_states.insert((depth, stencil), state);
        Ok(raw)
    }

    /// sampler state(+0, cache 소유).
    pub(crate) fn sampler(
        &mut self,
        device: &Id,
        desc: SamplerDesc,
    ) -> Result<RawId, BackendError> {
        if let Some(state) = self.samplers.get(&desc) {
            return Ok(state.as_raw());
        }
        let state = create_sampler(device, desc)?;
        let raw = state.as_raw();
        self.samplers.insert(desc, state);
        Ok(raw)
    }
}

/// `+new`로 descriptor 객체를 만듭니다.
fn new_object(class: Option<crate::objc::Class>) -> Result<Id, BackendError> {
    let class = class.ok_or(BackendError::new(
        BackendErrorKind::Unsupported,
        "Metal descriptor class가 없습니다",
    ))?;
    // SAFETY: `+new`는 () -> id(+1)입니다.
    unsafe { Id::from_owned(send(class.as_raw(), sel!("new"), ())) }.ok_or(BackendError::new(
        BackendErrorKind::OutOfHostMemory,
        "Metal descriptor를 만들 수 없습니다",
    ))
}

/// pipeline descriptor를 채우고 state를 만듭니다.
fn create_state(
    device: &Id,
    key: &PipelineKey,
    depth_format: Option<TextureFormat>,
    layout: &ProgramLayout,
    vertex: &Id,
    fragment: &Id,
) -> Result<Id, ShaderError> {
    let state_error = |log| ShaderError {
        stage: ShaderErrorStage::PipelineState,
        program: Some(key.program),
        log,
    };
    let descriptor = new_object(class!("MTLRenderPipelineDescriptor"))
        .map_err(|_| state_error(String::new()))?;
    // SAFETY: descriptor setter: setVertexFunction:/setFragmentFunction:(id),
    // setRasterSampleCount:/setDepthAttachmentPixelFormat:/setStencilAttachmentPixelFormat:
    // (NSUInteger), colorAttachments → objectAtIndexedSubscript:(0) → setPixelFormat:/setWriteMask:
    // (NSUInteger), setBlendingEnabled:(BOOL), set*BlendFactor:(NSUInteger).
    // `-newRenderPipelineStateWithDescriptor:error:`는 (descriptor, NSError**) -> id(+1, nil 가능).
    unsafe {
        let _: () = descriptor.send(sel!("setVertexFunction:"), (vertex.as_raw(),));
        let _: () = descriptor.send(sel!("setFragmentFunction:"), (fragment.as_raw(),));
        let _: () = descriptor.send(sel!("setRasterSampleCount:"), (sample_count(key.samples),));
        if let Some(format) = depth_format {
            let format = mtl_pixel_format(format);
            let _: () = descriptor.send(sel!("setDepthAttachmentPixelFormat:"), (format,));
            let _: () = descriptor.send(sel!("setStencilAttachmentPixelFormat:"), (format,));
        }
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
        if !layout.writes_color {
            let _: () = send(color, sel!("setWriteMask:"), (WRITE_MASK_NONE,));
        }
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
        Id::from_owned(state).ok_or_else(|| state_error(error_log(error)))
    }
}

/// stencil 연산: (비교, 통과 시 연산).
const fn stencil_ops(stencil: StencilMode) -> Option<(NSUInteger, NSUInteger)> {
    match stencil {
        StencilMode::Disabled => None,
        StencilMode::IncrementWhereEqual => Some((COMPARE_EQUAL, STENCIL_INCREMENT_CLAMP)),
        // reference + 1 비교는 backend가 reference 값을 1 올려 맞춥니다(commands 모듈).
        StencilMode::DecrementWhereEqual => Some((COMPARE_EQUAL, STENCIL_DECREMENT_CLAMP)),
        StencilMode::TestEqual => Some((COMPARE_EQUAL, STENCIL_KEEP)),
    }
}

fn create_depth_stencil(
    device: &Id,
    depth: DepthMode,
    stencil: StencilMode,
) -> Result<Id, BackendError> {
    let descriptor = new_object(class!("MTLDepthStencilDescriptor"))?;
    let (compare, write) = match depth {
        DepthMode::Disabled => (COMPARE_ALWAYS, false),
        DepthMode::TestAndWrite => (COMPARE_LESS, true),
        DepthMode::TestOnly => (COMPARE_LESS, false),
    };
    // SAFETY: depth-stencil descriptor setter: setDepthCompareFunction:(NSUInteger),
    // setDepthWriteEnabled:(BOOL), setFrontFaceStencil:/setBackFaceStencil:(MTLStencilDescriptor*).
    // stencil descriptor setter는 (NSUInteger) 또는 (uint32_t) mask입니다.
    // `-newDepthStencilStateWithDescriptor:`는 (descriptor) -> id(+1, nil 가능).
    unsafe {
        let _: () = descriptor.send(sel!("setDepthCompareFunction:"), (compare,));
        let _: () = descriptor.send(sel!("setDepthWriteEnabled:"), (write,));
        if let Some((compare, pass)) = stencil_ops(stencil) {
            let face = new_object(class!("MTLStencilDescriptor"))?;
            let _: () = face.send(sel!("setStencilCompareFunction:"), (compare,));
            let _: () = face.send(sel!("setStencilFailureOperation:"), (STENCIL_KEEP,));
            let _: () = face.send(sel!("setDepthFailureOperation:"), (STENCIL_KEEP,));
            let _: () = face.send(sel!("setDepthStencilPassOperation:"), (pass,));
            let _: () = face.send(sel!("setReadMask:"), (0xFF_u32,));
            let _: () = face.send(sel!("setWriteMask:"), (0xFF_u32,));
            let _: () = descriptor.send(sel!("setFrontFaceStencil:"), (face.as_raw(),));
            let _: () = descriptor.send(sel!("setBackFaceStencil:"), (face.as_raw(),));
        }
        Id::from_owned(device.send(
            sel!("newDepthStencilStateWithDescriptor:"),
            (descriptor.as_raw(),),
        ))
    }
    .ok_or(BackendError::new(
        BackendErrorKind::PipelineCreation,
        "depth-stencil state 생성 실패",
    ))
}

fn create_sampler(device: &Id, desc: SamplerDesc) -> Result<Id, BackendError> {
    let descriptor = new_object(class!("MTLSamplerDescriptor"))?;
    let filter = match desc.filter {
        TextureFilter::Nearest => FILTER_NEAREST,
        TextureFilter::Linear => FILTER_LINEAR,
    };
    let address = match desc.wrap {
        TextureWrap::ClampToEdge => ADDRESS_CLAMP,
        TextureWrap::Repeat => ADDRESS_REPEAT,
    };
    let mip = if desc.mipmaps { MIP_LINEAR } else { MIP_NONE };
    // SAFETY: sampler descriptor setter는 모두 (NSUInteger) -> void이고,
    // `-newSamplerStateWithDescriptor:`는 (descriptor) -> id(+1, nil 가능)입니다.
    unsafe {
        let _: () = descriptor.send(sel!("setMinFilter:"), (filter,));
        let _: () = descriptor.send(sel!("setMagFilter:"), (filter,));
        let _: () = descriptor.send(sel!("setMipFilter:"), (mip,));
        let _: () = descriptor.send(sel!("setSAddressMode:"), (address,));
        let _: () = descriptor.send(sel!("setTAddressMode:"), (address,));
        Id::from_owned(device.send(
            sel!("newSamplerStateWithDescriptor:"),
            (descriptor.as_raw(),),
        ))
    }
    .ok_or(BackendError::new(
        BackendErrorKind::OutOfDeviceMemory,
        "sampler state 생성 실패",
    ))
}

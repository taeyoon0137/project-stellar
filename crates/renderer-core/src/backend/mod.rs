//! GPU backend interface (XI.5.1).
//!
//! Metal·D3D12는 [`RendererBackend`] 뒤에 격리합니다(아키텍처 §10.1). 공통 계층은 trait 메서드에
//! descriptor·handle·[`CompiledRenderGraph`]·[`DrawCommand`] 값만 넘기고 platform GPU 객체를 보지
//! 않습니다. native surface(CAMetalLayer·swap chain)는 backend 생성 시 platform 코드가 넘기므로
//! trait에 나타나지 않습니다.
//!
//! ## 호출 순서 계약
//!
//! ```text
//! init ─▶ { resize | create_* | update_* | destroy_* | poll | query_timestamps }*
//!      ─▶ encode ─┬─▶ (Skipped: frame 없음, 다시 대기)
//!                 └─▶ Encoded ─▶ [schedule_present] ─▶ submit ─▶ (대기로 복귀)
//! ```
//!
//! - `init`이 가장 먼저이며 한 번입니다. device lost 뒤에는 다시 `init`만 허용합니다.
//! - frame은 한 번에 하나만 열립니다. `encode`가 `Encoded`를 돌려준 뒤 `submit` 전까지는 resize와
//!   resource 생성·갱신·파괴를 금지합니다(upload는 encode 전에 끝냄, XI.5.3).
//! - present 예약은 `submit` **전에** 열린 frame에 한 번 겁니다. Metal은 command buffer에 present를
//!   붙이고, D3D12는 execute 뒤 Present를 예약합니다. 따라서 `submit`이 끝나면 GPU 제출과 present
//!   예약이 모두 끝납니다(아키텍처 §6.1 T5). surface에 그리지 않는 offscreen frame은 present하지
//!   않습니다.
//! - frame 번호와 submission 번호는 엄격히 증가합니다.
//!
//! [`CheckedBackend`]가 이 계약을 실행 시 검사하는 wrapper입니다. 위반 호출은 내부 backend에
//! 닿기 전에 [`BackendErrorKind::InvalidUsage`]로 거부됩니다.
//!
//! ## handle
//!
//! [`BufferHandle`]·[`TextureHandle`]은 generational ID입니다([`crate::gpu_handle`]). backend는
//! native 객체를 [`HandleTable`](crate::gpu_handle::HandleTable)에 보관하고 handle을 그 table로만
//! 해석합니다. 파괴된 resource의 handle은 stale로 실패하며(`From<HandleError> for BackendError`),
//! device lost 뒤 새 device의 table은 epoch를 올려 이전 handle을 모두 무효로 만듭니다.

use std::fmt;

use stellar_scene::ColorSpace;

use crate::render_graph::{
    CompiledRenderGraph, ImportKind, PassId, ResourceId, ResourceLifetime, SampleCount,
    TextureFormat,
};

/// pipeline key와 draw command.
pub mod command;

/// 호출 순서 검사.
mod call_order;

/// GPU resource handle은 generational ID입니다(XI.5.2, [`crate::gpu_handle`]).
pub use crate::gpu_handle::{BufferHandle, TextureHandle};
pub use call_order::{
    BackendCall, CallOrderError, CallOrderState, CallOrderValidator, CheckedBackend,
};
pub use command::{
    BlendState, DepthMode, DrawCommand, PassCommands, PipelineKey, ScissorRect, ShaderProgram,
    StencilMode, TextureSource,
};

use crate::gpu_handle::HandleError;

/// 동시에 GPU에 올라갈 수 있는 frame 최대 수.
pub const MAX_FRAMES_IN_FLIGHT: u8 = 3;

/// backend 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// macOS Metal.
    Metal,
    /// Windows Direct3D 12.
    Direct3D12,
    /// 테스트·headless용 가짜 backend.
    Mock,
}

/// surface pixel 크기. 0 크기(최소화·가려짐)도 허용하며 그 동안 frame은 skip합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceSize {
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
}

impl SurfaceSize {
    /// 그릴 수 없는 크기면 참입니다.
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// surface 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceConfig {
    /// pixel 크기.
    pub size: SurfaceSize,
    /// drawable format.
    pub format: TextureFormat,
    /// surface 색공간(Display P3 지원 여부는 capability, XI.10.5).
    pub color_space: ColorSpace,
}

/// device 초기화 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceConfig {
    /// surface 설정.
    pub surface: SurfaceConfig,
    /// frames in flight(1..=[`MAX_FRAMES_IN_FLIGHT`]).
    pub frames_in_flight: u8,
    /// GPU timestamp query 사용 여부.
    pub timestamps: bool,
}

/// device capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceCapabilities {
    /// backend 종류.
    pub backend: BackendKind,
    /// texture 한 변 최대 pixel.
    pub max_texture_dimension: u32,
    /// Display P3 surface 지원.
    pub display_p3_surface: bool,
    /// GPU timestamp query 지원.
    pub timestamp_query: bool,
    /// 4x MSAA 지원.
    pub msaa_x4: bool,
    /// stencil bit 수(0이면 stencil 없음).
    pub stencil_bits: u8,
}

/// buffer 용도.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BufferUsage {
    /// vertex buffer.
    Vertex,
    /// index buffer(u32).
    Index,
    /// uniform buffer.
    Uniform,
    /// CPU → GPU upload staging.
    Staging,
}

/// GPU buffer descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferDesc {
    /// byte 크기(양수).
    pub size: u64,
    /// 용도.
    pub usage: BufferUsage,
}

/// texture 용도.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureUsage {
    /// shader에서 sampling.
    pub sampled: bool,
    /// render target으로 그림.
    pub render_target: bool,
}

/// GPU texture descriptor(graph 밖 수명의 texture: 이미지·atlas·placeholder).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GpuTextureDesc {
    /// 너비(양수).
    pub width: u32,
    /// 높이(양수).
    pub height: u32,
    /// format.
    pub format: TextureFormat,
    /// mip level 수(1 이상).
    pub mip_levels: u32,
    /// multisample 수.
    pub samples: SampleCount,
    /// 용도.
    pub usage: TextureUsage,
}

/// texture 갱신 영역.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureRegion {
    /// mip level.
    pub mip_level: u32,
    /// 왼쪽.
    pub x: u32,
    /// 위.
    pub y: u32,
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
}

/// frame 번호(엄격 증가).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameId(pub u64);

/// GPU 제출 번호(backend가 엄격 증가로 발급).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubmissionId(pub u64);

/// imported graph resource를 backend 대상에 묶습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportTarget {
    /// 현재 surface drawable(`ImportKind::Surface` 전용).
    Surface,
    /// graph 밖 texture(`ImportKind::External` 전용).
    Texture(TextureHandle),
}

/// imported resource 하나의 binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImportBinding {
    /// graph resource.
    pub resource: ResourceId,
    /// 대상.
    pub target: ImportTarget,
}

/// frame encode 요청.
#[derive(Debug, Clone, Copy)]
pub struct FrameEncodeRequest<'a> {
    /// frame 번호.
    pub frame: FrameId,
    /// 검증된 graph.
    pub graph: &'a CompiledRenderGraph,
    /// pass별 command(graph에 있는 pass만, pass마다 최대 하나).
    pub passes: &'a [PassCommands],
    /// imported resource binding(graph의 imported resource마다 하나).
    pub imports: &'a [ImportBinding],
}

impl FrameEncodeRequest<'_> {
    /// graph output이 surface면 참입니다(present 대상 frame).
    pub fn targets_surface(&self) -> bool {
        self.graph
            .output()
            .and_then(|output| self.graph.resource(output.resource))
            .is_some_and(|resource| {
                resource.lifetime == ResourceLifetime::Imported(ImportKind::Surface)
            })
    }

    /// pass command 목록을 graph에서 찾습니다.
    pub fn commands_for(&self, pass: PassId) -> Option<&PassCommands> {
        self.passes.iter().find(|commands| commands.pass == pass)
    }
}

/// encode하지 않고 frame을 건너뛴 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkipReason {
    /// surface 크기가 0입니다(최소화).
    EmptySurface,
    /// drawable을 얻지 못했습니다(가려짐·timeout).
    NoDrawable,
}

/// backend가 encode한 frame. native command buffer는 backend 안에 있고 이 값은 token입니다.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct EncodedFrame {
    frame: FrameId,
    token: u64,
}

impl EncodedFrame {
    /// backend가 encode 결과 token을 감쌉니다.
    pub const fn new(frame: FrameId, token: u64) -> EncodedFrame {
        EncodedFrame { frame, token }
    }

    /// frame 번호.
    pub const fn frame(&self) -> FrameId {
        self.frame
    }

    /// backend token.
    pub const fn token(&self) -> u64 {
        self.token
    }
}

/// encode 결과.
#[derive(Debug, PartialEq, Eq)]
pub enum EncodeOutcome {
    /// encode 완료. `submit`해야 합니다.
    Encoded(EncodedFrame),
    /// frame을 건너뜀(오류 아님).
    Skipped(SkipReason),
}

/// GPU 진행 상황.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct GpuProgress {
    /// GPU 실행이 끝난 가장 큰 submission.
    pub completed: Option<SubmissionId>,
    /// 화면에 present된 가장 큰 submission.
    pub presented: Option<SubmissionId>,
}

/// pass 하나의 GPU 실행 시간.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PassGpuTime {
    /// pass.
    pub pass: PassId,
    /// GPU 실행 시간(ns).
    pub nanos: u64,
}

/// submission 하나의 GPU timestamp 결과.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GpuTimings {
    /// pass별 시간(실행 순서).
    pub passes: Vec<PassGpuTime>,
    /// submission 전체 GPU 시간(ns).
    pub total_nanos: u64,
}

/// timestamp query 결과.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TimestampQuery {
    /// GPU가 아직 끝나지 않았습니다.
    Pending,
    /// 결과.
    Ready(GpuTimings),
    /// device 또는 설정이 timestamp를 지원하지 않습니다.
    Unsupported,
}

/// backend 오류 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendErrorKind {
    /// 호출 순서·인자 위반(공통 계층 버그).
    InvalidUsage,
    /// device·driver가 기능을 지원하지 않습니다.
    Unsupported,
    /// GPU device를 잃었습니다(driver reset, eGPU 분리 등).
    DeviceLost,
    /// GPU 메모리 부족.
    OutOfDeviceMemory,
    /// CPU 메모리 부족.
    OutOfHostMemory,
    /// surface를 잃었거나 다시 만들어야 합니다.
    SurfaceLost,
    /// drawable·fence 대기 timeout.
    Timeout,
    /// shader·pipeline 생성 실패.
    PipelineCreation,
    /// 분류되지 않은 backend 내부 오류.
    Internal,
}

/// backend 오류. 정적 detail만 담아 진단에 사용자 내용이 섞이지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BackendError {
    kind: BackendErrorKind,
    detail: &'static str,
    call_order: Option<CallOrderError>,
}

impl BackendError {
    /// 오류를 만듭니다.
    pub const fn new(kind: BackendErrorKind, detail: &'static str) -> BackendError {
        BackendError {
            kind,
            detail,
            call_order: None,
        }
    }

    /// 호출 순서 위반 오류.
    pub const fn call_order(error: CallOrderError) -> BackendError {
        BackendError {
            kind: BackendErrorKind::InvalidUsage,
            detail: "backend 호출 순서 위반",
            call_order: Some(error),
        }
    }

    /// 종류.
    pub const fn kind(&self) -> BackendErrorKind {
        self.kind
    }

    /// 정적 detail.
    pub const fn detail(&self) -> &'static str {
        self.detail
    }

    /// 호출 순서 위반이면 그 내용.
    pub const fn call_order_violation(&self) -> Option<CallOrderError> {
        self.call_order
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.detail)?;
        if let Some(violation) = self.call_order {
            write!(f, " ({violation})")?;
        }
        Ok(())
    }
}

impl std::error::Error for BackendError {}

impl From<HandleError> for BackendError {
    /// 만료·다른 device·범위 밖 handle 사용은 공통 계층의 호출 오류입니다.
    fn from(error: HandleError) -> BackendError {
        let detail = match error {
            HandleError::WrongEpoch { .. } => "이전 device 세대의 GPU handle",
            HandleError::Stale { .. } => "파괴된 GPU handle",
            HandleError::OutOfRange { .. } => "없는 GPU handle",
            HandleError::Exhausted => "GPU handle 공간 소진",
        };
        let kind = match error {
            HandleError::Exhausted => BackendErrorKind::OutOfHostMemory,
            _ => BackendErrorKind::InvalidUsage,
        };
        BackendError::new(kind, detail)
    }
}

/// platform GPU backend.
///
/// 구현은 모듈 문서의 호출 순서 계약을 가정해도 됩니다. 공통 계층은 [`CheckedBackend`]로 감싸
/// 계약을 검사합니다. 모든 메서드는 실패를 [`BackendError`]로 돌려주며 panic하지 않습니다.
pub trait RendererBackend {
    /// backend 종류.
    fn kind(&self) -> BackendKind;

    /// device·queue·surface를 초기화합니다.
    fn init(&mut self, config: &DeviceConfig) -> Result<DeviceCapabilities, BackendError>;

    /// surface 크기를 바꿉니다(0 크기 허용).
    fn resize(&mut self, size: SurfaceSize) -> Result<(), BackendError>;

    /// buffer를 만듭니다.
    fn create_buffer(&mut self, desc: &BufferDesc) -> Result<BufferHandle, BackendError>;

    /// texture를 만듭니다.
    fn create_texture(&mut self, desc: &GpuTextureDesc) -> Result<TextureHandle, BackendError>;

    /// buffer 일부를 갱신합니다.
    fn update_buffer(
        &mut self,
        buffer: BufferHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BackendError>;

    /// texture 영역을 갱신합니다(RGBA8은 행 padding 없는 `width × 4` stride).
    fn update_texture(
        &mut self,
        texture: TextureHandle,
        region: TextureRegion,
        data: &[u8],
    ) -> Result<(), BackendError>;

    /// buffer를 파괴합니다.
    ///
    /// 공통 계층은 이 buffer를 참조한 마지막 submission이 GPU에서 끝난 뒤에만 호출합니다
    /// ([`DeferredRelease`](crate::frame_resources::DeferredRelease), XI.5.3). backend는 즉시
    /// 해제해도 됩니다.
    fn destroy_buffer(&mut self, buffer: BufferHandle) -> Result<(), BackendError>;

    /// texture를 파괴합니다. 호출 시점 계약은 [`RendererBackend::destroy_buffer`]와 같습니다.
    fn destroy_texture(&mut self, texture: TextureHandle) -> Result<(), BackendError>;

    /// graph를 command buffer로 encode합니다. surface frame이면 drawable을 얻습니다.
    fn encode(&mut self, request: &FrameEncodeRequest<'_>) -> Result<EncodeOutcome, BackendError>;

    /// 열린 frame에 present를 예약합니다(submit 전).
    fn schedule_present(&mut self, frame: &EncodedFrame) -> Result<(), BackendError>;

    /// encode한 frame을 GPU queue에 제출합니다. 예약한 present도 이때 확정됩니다.
    fn submit(&mut self, frame: EncodedFrame) -> Result<SubmissionId, BackendError>;

    /// GPU 완료·present 진행을 조회합니다(대기하지 않음).
    fn poll(&mut self) -> Result<GpuProgress, BackendError>;

    /// submission의 GPU timestamp를 조회합니다(대기하지 않음).
    fn query_timestamps(
        &mut self,
        submission: SubmissionId,
    ) -> Result<TimestampQuery, BackendError>;
}

#[cfg(test)]
pub(crate) mod mock;

#[cfg(test)]
pub(crate) mod tests;

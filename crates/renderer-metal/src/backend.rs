//! [`RendererBackend`]의 Metal 구현.
//!
//! - surface가 있는 backend([`MetalBackend::with_layer`])는 CAMetalLayer drawable에 그리고 present를
//!   예약합니다. surface 없는 backend([`MetalBackend::headless`])는 external texture에만 그립니다
//!   (offscreen 적합성 harness·테스트).
//! - submission 번호는 device마다 1부터 셉니다. GPU 진행은 command buffer `status`로 비대기 조회합니다.
//!   단일 queue이므로 앞 submission이 끝나기 전 뒤 submission을 완료로 보지 않습니다.
//! - present 확정 시점은 present를 붙인 command buffer의 GPU 완료입니다. 실제 화면 표시 시각은 창
//!   상태·compositor에 따라 다르고(창 밖 layer는 `presentedTime`이 오지 않음) 아키텍처 §6.1에서 별도
//!   관측 지표입니다.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr::NonNull;

use stellar_renderer_core::backend::{
    BackendError, BackendErrorKind, BackendKind, BufferDesc, BufferHandle, DeviceCapabilities,
    DeviceConfig, EncodeOutcome, EncodedFrame, FrameEncodeRequest, FrameId, GpuProgress,
    GpuTextureDesc, RendererBackend, SkipReason, SubmissionId, SurfaceSize, TextureHandle,
    TextureRegion, TimestampQuery, MAX_FRAMES_IN_FLIGHT,
};
use stellar_renderer_core::gpu_handle::{BufferKind, HandleTable, TextureKind};
use stellar_renderer_core::render_graph::{
    ImportKind, ResourceLifetime, SampleCount, TextureFormat,
};

use crate::buffer::{stage_texture_upload, submit_uploads, MetalBuffer, PendingUpload};
use crate::commands::CommandContext;
use crate::device::MetalDevice;
use crate::encode::{encode_pass, plan_pass, FrameTargets};
use crate::format::bytes_per_pixel;
use crate::objc::{autoreleasepool, sel, Id, NSUInteger, RawId};
use crate::pipeline::{PipelineCache, ShaderError};
use crate::surface::{check_surface_size, Drawable, LayerOptions, LayerSurface, SurfaceStats};
use crate::sys::{command_buffer_status, resource_options, MTLOrigin, MTLSize};
use crate::texture::{create_texture, MetalTexture};

/// encode했지만 아직 제출하지 않은 frame.
#[derive(Debug)]
struct OpenFrame {
    frame: FrameId,
    token: u64,
    command_buffer: Id,
    drawable: Option<Drawable>,
    present: bool,
}

/// GPU에 제출한 command buffer. texture upload command buffer는 submission 번호가 없습니다.
#[derive(Debug)]
struct InFlight {
    id: Option<SubmissionId>,
    command_buffer: Id,
    present: bool,
}

/// macOS Metal backend.
#[derive(Debug)]
pub struct MetalBackend {
    surface: Option<LayerSurface>,
    device: Option<MetalDevice>,
    capabilities: Option<DeviceCapabilities>,
    surface_size: SurfaceSize,
    surface_format: TextureFormat,
    textures: HandleTable<TextureKind, MetalTexture>,
    buffers: HandleTable<BufferKind, MetalBuffer>,
    pipelines: PipelineCache,
    pending_uploads: Vec<PendingUpload>,
    initialized_once: bool,
    open: Option<OpenFrame>,
    next_token: u64,
    next_submission: u64,
    in_flight: VecDeque<InFlight>,
    progress: GpuProgress,
    surface_stats: SurfaceStats,
    /// 테스트용: 다음 n번의 drawable 요청을 nil로 만듭니다(가려진 창·timeout 재현).
    #[cfg(test)]
    pub(crate) drawable_failures: u32,
}

fn invalid(detail: &'static str) -> BackendError {
    BackendError::new(BackendErrorKind::InvalidUsage, detail)
}

impl MetalBackend {
    fn new(surface: Option<LayerSurface>) -> MetalBackend {
        MetalBackend {
            surface,
            device: None,
            capabilities: None,
            surface_size: SurfaceSize {
                width: 0,
                height: 0,
            },
            surface_format: TextureFormat::Bgra8Unorm,
            textures: HandleTable::new(0),
            buffers: HandleTable::new(0),
            pipelines: PipelineCache::new(),
            pending_uploads: Vec::new(),
            initialized_once: false,
            open: None,
            next_token: 1,
            next_submission: 1,
            in_flight: VecDeque::new(),
            progress: GpuProgress::default(),
            surface_stats: SurfaceStats::default(),
            #[cfg(test)]
            drawable_failures: 0,
        }
    }

    /// surface 없는 backend. external texture에만 그리며 present하지 않습니다.
    pub fn headless() -> MetalBackend {
        MetalBackend::new(None)
    }

    /// platform 셸의 CAMetalLayer에 그리는 backend. layer를 retain합니다.
    ///
    /// device·pixel format·drawable 크기는 [`RendererBackend::init`]에서 설정합니다.
    ///
    /// # Safety
    ///
    /// `layer`는 살아 있는 Objective-C 객체(`CAMetalLayer`)를 가리켜야 합니다. CAMetalLayer가 아닌
    /// 객체면 `InvalidUsage`를 돌려줍니다.
    pub unsafe fn with_layer(
        layer: NonNull<c_void>,
        options: LayerOptions,
    ) -> Result<MetalBackend, BackendError> {
        // SAFETY: layer 유효성은 호출자 보장입니다.
        let surface = autoreleasepool(|| unsafe { LayerSurface::from_raw(layer, options) })?;
        Ok(MetalBackend::new(Some(surface)))
    }

    /// 창에 붙지 않은 새 CAMetalLayer에 그리는 backend(테스트·진단용).
    pub fn with_detached_layer(options: LayerOptions) -> Result<MetalBackend, BackendError> {
        let surface = autoreleasepool(|| LayerSurface::detached(options))?;
        Ok(MetalBackend::new(Some(surface)))
    }

    /// surface(CAMetalLayer)가 있으면 참입니다.
    pub fn has_surface(&self) -> bool {
        self.surface.is_some()
    }

    /// surface resize·frame skip 통계.
    pub fn surface_stats(&self) -> SurfaceStats {
        self.surface_stats
    }

    /// 현재 surface pixel 크기(마지막 `init`·`resize` 값).
    pub fn surface_size(&self) -> SurfaceSize {
        self.surface_size
    }

    /// 마지막 shader compile·pipeline 생성 오류(단계와 Metal compiler log). `PipelineCreation` 오류의
    /// 개발자 context입니다.
    pub fn last_shader_error(&self) -> Option<ShaderError> {
        self.pipelines.last_error().cloned()
    }

    /// 현재 device에서 MSL library를 실제로 compile한 횟수(cache 적중은 세지 않음, 실패 포함).
    pub fn shader_compile_count(&self) -> u64 {
        self.pipelines.library_compiles()
    }

    /// init이 돌려준 capability(init 전이면 `None`).
    pub fn capabilities(&self) -> Option<DeviceCapabilities> {
        self.capabilities
    }

    fn device(&self) -> Result<&MetalDevice, BackendError> {
        self.device
            .as_ref()
            .ok_or(invalid("Metal backend가 초기화되지 않았습니다"))
    }

    /// GPU에 남은 submission이 모두 끝날 때까지 기다리고 진행을 돌려줍니다(blocking).
    ///
    /// 종료·device 재생성·테스트용입니다. frame loop에서는 [`RendererBackend::poll`]을 씁니다.
    pub fn wait_idle(&mut self) -> Result<GpuProgress, BackendError> {
        autoreleasepool(|| self.flush_uploads())?;
        if let Some(last) = self.in_flight.back() {
            // SAFETY: `-waitUntilCompleted`는 () -> void이고 commit한 command buffer에만 부릅니다.
            unsafe {
                let _: () = last.command_buffer.send(sel!("waitUntilCompleted"), ());
            }
        }
        self.poll_completions()
    }

    /// texture mip 0 내용을 CPU로 읽습니다(blocking, format 그대로의 byte 순서, 행 padding 없음).
    ///
    /// 적합성 harness(XI.11.1)·테스트용입니다. 앞서 제출한 모든 GPU 작업 뒤에 읽습니다.
    /// multisample·depth texture는 읽을 수 없습니다(`InvalidUsage`).
    pub fn read_texture(&mut self, texture: TextureHandle) -> Result<Vec<u8>, BackendError> {
        autoreleasepool(|| self.read_texture_inner(texture))
    }

    fn read_texture_inner(&mut self, texture: TextureHandle) -> Result<Vec<u8>, BackendError> {
        if self.open.is_some() {
            return Err(invalid("열린 frame이 있는 동안 readback할 수 없습니다"));
        }
        self.flush_uploads()?;
        let device = self.device()?;
        let native = self.textures.get(texture)?;
        let desc = native.desc;
        let bytes_per_pixel = bytes_per_pixel(desc.format)
            .filter(|_| desc.samples == SampleCount::X1)
            .ok_or(invalid(
                "multisample·depth texture는 readback할 수 없습니다",
            ))?;
        let row = u64::from(desc.width) * u64::from(bytes_per_pixel);
        let length = row * u64::from(desc.height);
        let length_usize =
            usize::try_from(length).map_err(|_| invalid("readback 크기가 너무 큽니다"))?;
        // SAFETY: `-newBufferWithLength:options:`는 (NSUInteger, NSUInteger) -> id<MTLBuffer>(+1,
        // nil 가능), `-commandBuffer`는 () -> id(+0), `-blitCommandEncoder`는 () -> id(+0)입니다.
        // copyFromTexture:...toBuffer: 인자는 (texture, slice, level, MTLOrigin, MTLSize, buffer,
        // offset, bytesPerRow, bytesPerImage)이고 범위는 texture 크기와 buffer 길이로 맞췄습니다.
        unsafe {
            let buffer = Id::from_owned(device.device().send(
                sel!("newBufferWithLength:options:"),
                (length_usize as NSUInteger, resource_options::STORAGE_SHARED),
            ))
            .ok_or(BackendError::new(
                BackendErrorKind::OutOfDeviceMemory,
                "readback buffer를 할당할 수 없습니다",
            ))?;
            let command_buffer = Id::from_borrowed(device.queue().send(sel!("commandBuffer"), ()))
                .ok_or(BackendError::new(
                    BackendErrorKind::Internal,
                    "command buffer를 만들 수 없습니다",
                ))?;
            let blit: RawId = command_buffer.send(sel!("blitCommandEncoder"), ());
            if blit.is_null() {
                return Err(BackendError::new(
                    BackendErrorKind::Internal,
                    "blit encoder를 만들 수 없습니다",
                ));
            }
            let _: () = crate::objc::send(
                blit,
                sel!("copyFromTexture:sourceSlice:sourceLevel:sourceOrigin:sourceSize:toBuffer:destinationOffset:destinationBytesPerRow:destinationBytesPerImage:"),
                (
                    native.texture.as_raw(),
                    0 as NSUInteger,
                    0 as NSUInteger,
                    MTLOrigin { x: 0, y: 0, z: 0 },
                    MTLSize {
                        width: desc.width as NSUInteger,
                        height: desc.height as NSUInteger,
                        depth: 1,
                    },
                    buffer.as_raw(),
                    0 as NSUInteger,
                    row as NSUInteger,
                    length_usize as NSUInteger,
                ),
            );
            let _: () = crate::objc::send(blit, sel!("endEncoding"), ());
            let _: () = command_buffer.send(sel!("commit"), ());
            let _: () = command_buffer.send(sel!("waitUntilCompleted"), ());
            let status: NSUInteger = command_buffer.send(sel!("status"), ());
            if status != command_buffer_status::COMPLETED {
                return Err(BackendError::new(
                    BackendErrorKind::Internal,
                    "readback command buffer 실행 실패",
                ));
            }
            let contents: *const u8 = buffer.send(sel!("contents"), ());
            if contents.is_null() {
                return Err(BackendError::new(
                    BackendErrorKind::Internal,
                    "readback buffer 내용에 접근할 수 없습니다",
                ));
            }
            Ok(std::slice::from_raw_parts(contents, length_usize).to_vec())
        }
    }

    /// 쌓인 texture upload를 blit command buffer로 제출합니다(frame보다 먼저 queue에 들어감).
    fn flush_uploads(&mut self) -> Result<(), BackendError> {
        if self.pending_uploads.is_empty() {
            return Ok(());
        }
        let device = self
            .device
            .as_ref()
            .ok_or(invalid("Metal backend가 초기화되지 않았습니다"))?;
        if let Some(command_buffer) = submit_uploads(device.queue(), &mut self.pending_uploads)? {
            self.in_flight.push_back(InFlight {
                id: None,
                command_buffer,
                present: false,
            });
        }
        Ok(())
    }

    /// 끝난 command buffer를 앞에서부터 거둡니다.
    fn poll_completions(&mut self) -> Result<GpuProgress, BackendError> {
        while let Some(front) = self.in_flight.front() {
            // SAFETY: `-status`는 () -> NSUInteger(MTLCommandBufferStatus)입니다.
            let status: NSUInteger = unsafe { front.command_buffer.send(sel!("status"), ()) };
            match status {
                command_buffer_status::COMPLETED => {
                    if let Some(done) = self.in_flight.pop_front() {
                        if let Some(id) = done.id {
                            self.progress.completed = Some(id);
                            if done.present {
                                self.progress.presented = Some(id);
                            }
                        }
                    }
                }
                command_buffer_status::ERROR => {
                    if let Some(done) = self.in_flight.pop_front() {
                        if let Some(id) = done.id {
                            self.progress.completed = Some(id);
                        }
                    }
                    return Err(BackendError::new(
                        BackendErrorKind::Internal,
                        "GPU command buffer 실행 실패",
                    ));
                }
                _ => break,
            }
        }
        Ok(self.progress)
    }

    /// 테스트용: 내장 shader 대신 `source`로 pipeline을 만듭니다(compile 실패 경로 재현).
    #[cfg(test)]
    pub(crate) fn use_shader_source(&mut self, source: &'static str) {
        self.pipelines = PipelineCache::with_source(source);
    }

    /// 테스트용: 만든 pipeline state 수.
    #[cfg(test)]
    pub(crate) fn pipeline_count(&self) -> usize {
        self.pipelines.len()
    }

    /// 테스트용: 연결된 layer.
    #[cfg(test)]
    pub(crate) fn layer_surface(&self) -> Option<&LayerSurface> {
        self.surface.as_ref()
    }

    /// 테스트용: 열린 surface frame의 drawable texture를 shared buffer로 복사하는 blit을 같은 command
    /// buffer에 붙입니다(render pass 뒤, present 전). layer가 `framebuffer_only = false`여야 합니다.
    /// GPU 완료 후 [`crate::tests::buffer_bytes`]로 읽습니다.
    #[cfg(test)]
    pub(crate) fn capture_open_drawable(&mut self) -> Result<(Id, usize), BackendError> {
        let device = self.device()?;
        let open = self.open.as_ref().ok_or(invalid("열린 frame 없음"))?;
        let drawable = open.drawable.as_ref().ok_or(invalid("drawable 없음"))?;
        let bytes_per_pixel = bytes_per_pixel(self.surface_format).ok_or(invalid("format"))?;
        let row = self.surface_size.width as usize * bytes_per_pixel as usize;
        let length = row * self.surface_size.height as usize;
        autoreleasepool(|| {
            // SAFETY: read_texture_inner와 같은 selector 시그니처이고, 범위는 drawable 크기로 맞췄습니다.
            unsafe {
                let buffer = Id::from_owned(device.device().send(
                    sel!("newBufferWithLength:options:"),
                    (length as NSUInteger, resource_options::STORAGE_SHARED),
                ))
                .ok_or(invalid("buffer"))?;
                let blit: RawId = open.command_buffer.send(sel!("blitCommandEncoder"), ());
                let _: () = crate::objc::send(
                    blit,
                    sel!("copyFromTexture:sourceSlice:sourceLevel:sourceOrigin:sourceSize:toBuffer:destinationOffset:destinationBytesPerRow:destinationBytesPerImage:"),
                    (
                        drawable.texture().as_raw(),
                        0 as NSUInteger,
                        0 as NSUInteger,
                        MTLOrigin { x: 0, y: 0, z: 0 },
                        MTLSize {
                            width: self.surface_size.width as NSUInteger,
                            height: self.surface_size.height as NSUInteger,
                            depth: 1,
                        },
                        buffer.as_raw(),
                        0 as NSUInteger,
                        row as NSUInteger,
                        length as NSUInteger,
                    ),
                );
                let _: () = crate::objc::send(blit, sel!("endEncoding"), ());
                Ok((buffer, length))
            }
        })
    }

    /// 다음 drawable. nil이거나 크기가 backend 크기와 다르면(layer를 다시 맞춘 뒤) `None`.
    fn acquire_drawable(&mut self) -> Option<Drawable> {
        #[cfg(test)]
        if self.drawable_failures > 0 {
            self.drawable_failures -= 1;
            return None;
        }
        let surface = self.surface.as_ref()?;
        let drawable = surface.next_drawable()?;
        if drawable.size() != (self.surface_size.width, self.surface_size.height) {
            self.surface_stats.drawable_size_resyncs += 1;
            surface.set_drawable_size(self.surface_size);
            return None;
        }
        Some(drawable)
    }

    fn take_open(&mut self, frame: &EncodedFrame) -> Result<OpenFrame, BackendError> {
        match self.open.take() {
            Some(open) if open.token == frame.token() && open.frame == frame.frame() => Ok(open),
            other => {
                self.open = other;
                Err(invalid("열린 frame과 다른 frame입니다"))
            }
        }
    }

    fn encode_inner(
        &mut self,
        request: &FrameEncodeRequest<'_>,
    ) -> Result<EncodeOutcome, BackendError> {
        self.device()?;
        if self.open.is_some() {
            return Err(invalid("이미 열린 frame이 있습니다"));
        }
        // upload는 frame skip 여부와 상관없이 지금 queue에 넣습니다(다음 frame보다 먼저 실행).
        self.flush_uploads()?;
        let needs_surface =
            request.graph.graph().resources.iter().any(|resource| {
                resource.lifetime == ResourceLifetime::Imported(ImportKind::Surface)
            });
        let drawable = if needs_surface {
            if self.surface.is_none() {
                return Err(invalid("surface 없는 backend에 surface graph"));
            }
            if self.surface_size.is_empty() {
                self.surface_stats.skipped_empty += 1;
                return Ok(EncodeOutcome::Skipped(SkipReason::EmptySurface));
            }
            match self.acquire_drawable() {
                Some(drawable) => Some(drawable),
                None => {
                    self.surface_stats.skipped_no_drawable += 1;
                    return Ok(EncodeOutcome::Skipped(SkipReason::NoDrawable));
                }
            }
        } else {
            None
        };
        let device = self
            .device
            .as_ref()
            .ok_or(invalid("Metal backend가 초기화되지 않았습니다"))?;
        let targets = FrameTargets {
            request,
            drawable: drawable.as_ref(),
            surface_size: self.surface_size,
            surface_format: self.surface_format,
            textures: &self.textures,
        };
        let mut commands = CommandContext {
            device: device.device(),
            buffers: &self.buffers,
            textures: &self.textures,
            pipelines: &mut self.pipelines,
        };
        let plans = request
            .graph
            .order()
            .iter()
            .map(|&pass| plan_pass(&targets, &mut commands, pass))
            .collect::<Result<Vec<_>, _>>()?;
        // SAFETY: `-commandBuffer`는 () -> id<MTLCommandBuffer>(+0, nil 가능)입니다.
        let command_buffer =
            unsafe { Id::from_borrowed(device.queue().send(sel!("commandBuffer"), ())) }.ok_or(
                BackendError::new(
                    BackendErrorKind::Internal,
                    "command buffer를 만들 수 없습니다",
                ),
            )?;
        for plan in &plans {
            encode_pass(&command_buffer, plan)?;
        }
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1);
        self.open = Some(OpenFrame {
            frame: request.frame,
            token,
            command_buffer,
            drawable,
            present: false,
        });
        Ok(EncodeOutcome::Encoded(EncodedFrame::new(
            request.frame,
            token,
        )))
    }
}

impl RendererBackend for MetalBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Metal
    }

    fn init(&mut self, config: &DeviceConfig) -> Result<DeviceCapabilities, BackendError> {
        autoreleasepool(|| {
            if config.frames_in_flight == 0 || config.frames_in_flight > MAX_FRAMES_IN_FLIGHT {
                return Err(invalid("frames in flight는 1..=3이어야 합니다"));
            }
            if config.surface.format.is_depth_stencil() {
                return Err(invalid("surface format은 color format이어야 합니다"));
            }
            check_surface_size(config.surface.size)?;
            // 새 device: 이전 resource·frame·submission은 모두 버리고 이전 handle은 무효가 됩니다.
            self.open = None;
            self.in_flight.clear();
            self.device = None;
            self.capabilities = None;
            self.pending_uploads.clear();
            self.pipelines = PipelineCache::new();
            if self.initialized_once {
                self.textures.invalidate_all()?;
                self.buffers.invalidate_all()?;
            }
            self.initialized_once = true;
            let device = MetalDevice::system_default()?;
            if let Some(surface) = &self.surface {
                surface.configure(device.device(), &config.surface)?;
            }
            let capabilities = device.capabilities(self.surface.is_some(), false);
            self.surface_size = config.surface.size;
            self.surface_format = config.surface.format;
            self.next_submission = 1;
            self.progress = GpuProgress::default();
            self.device = Some(device);
            self.capabilities = Some(capabilities);
            Ok(capabilities)
        })
    }

    fn resize(&mut self, size: SurfaceSize) -> Result<(), BackendError> {
        autoreleasepool(|| {
            self.device()?;
            if self.open.is_some() {
                return Err(invalid("열린 frame이 있는 동안 resize할 수 없습니다"));
            }
            check_surface_size(size)?;
            if size == self.surface_size {
                return Ok(());
            }
            if let Some(surface) = &self.surface {
                surface.set_drawable_size(size);
            }
            self.surface_size = size;
            self.surface_stats.resizes += 1;
            Ok(())
        })
    }

    fn create_buffer(&mut self, desc: &BufferDesc) -> Result<BufferHandle, BackendError> {
        autoreleasepool(|| {
            if self.open.is_some() {
                return Err(invalid(
                    "열린 frame이 있는 동안 resource를 만들 수 없습니다",
                ));
            }
            let buffer = MetalBuffer::create(self.device()?.device(), desc)?;
            Ok(self.buffers.insert(buffer)?)
        })
    }

    fn create_texture(&mut self, desc: &GpuTextureDesc) -> Result<TextureHandle, BackendError> {
        autoreleasepool(|| {
            if self.open.is_some() {
                return Err(invalid(
                    "열린 frame이 있는 동안 resource를 만들 수 없습니다",
                ));
            }
            let texture = create_texture(self.device()?.device(), desc)?;
            Ok(self.textures.insert(MetalTexture {
                texture,
                desc: *desc,
            })?)
        })
    }

    fn update_buffer(
        &mut self,
        buffer: BufferHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BackendError> {
        if self.open.is_some() {
            return Err(invalid(
                "열린 frame이 있는 동안 resource를 갱신할 수 없습니다",
            ));
        }
        self.buffers.get_mut(buffer)?.write(offset, data)
    }

    fn update_texture(
        &mut self,
        texture: TextureHandle,
        region: TextureRegion,
        data: &[u8],
    ) -> Result<(), BackendError> {
        autoreleasepool(|| {
            if self.open.is_some() {
                return Err(invalid(
                    "열린 frame이 있는 동안 resource를 갱신할 수 없습니다",
                ));
            }
            let device = self
                .device
                .as_ref()
                .ok_or(invalid("Metal backend가 초기화되지 않았습니다"))?;
            let native = self.textures.get(texture)?;
            let upload = stage_texture_upload(device.device(), native, region, data)?;
            self.pending_uploads.push(upload);
            Ok(())
        })
    }

    fn destroy_buffer(&mut self, buffer: BufferHandle) -> Result<(), BackendError> {
        if self.open.is_some() {
            return Err(invalid(
                "열린 frame이 있는 동안 resource를 파괴할 수 없습니다",
            ));
        }
        // command buffer가 참조 buffer를 retain하므로 GPU 실행 중이어도 즉시 해제해도 안전합니다.
        autoreleasepool(|| self.buffers.remove(buffer).map(drop))?;
        Ok(())
    }

    fn destroy_texture(&mut self, texture: TextureHandle) -> Result<(), BackendError> {
        if self.open.is_some() {
            return Err(invalid(
                "열린 frame이 있는 동안 resource를 파괴할 수 없습니다",
            ));
        }
        // command buffer가 참조 texture를 retain하므로 GPU 실행 중이어도 즉시 해제해도 안전합니다.
        autoreleasepool(|| self.textures.remove(texture).map(drop))?;
        Ok(())
    }

    fn encode(&mut self, request: &FrameEncodeRequest<'_>) -> Result<EncodeOutcome, BackendError> {
        autoreleasepool(|| self.encode_inner(request))
    }

    fn schedule_present(&mut self, frame: &EncodedFrame) -> Result<(), BackendError> {
        let open = self
            .open
            .as_mut()
            .filter(|open| open.token == frame.token() && open.frame == frame.frame())
            .ok_or(invalid("열린 frame과 다른 frame입니다"))?;
        if open.present {
            return Err(invalid("present를 이미 예약했습니다"));
        }
        let drawable = open.drawable.as_ref().ok_or(invalid(
            "surface에 그리지 않는 frame은 present할 수 없습니다",
        ))?;
        autoreleasepool(|| {
            // SAFETY: `-presentDrawable:`은 (id<MTLDrawable>) -> void이고 commit 전 command buffer에
            // 붙입니다.
            unsafe {
                let _: () = open
                    .command_buffer
                    .send(sel!("presentDrawable:"), (drawable.drawable().as_raw(),));
            }
        });
        open.present = true;
        Ok(())
    }

    fn submit(&mut self, frame: EncodedFrame) -> Result<SubmissionId, BackendError> {
        let open = self.take_open(&frame)?;
        autoreleasepool(|| {
            // SAFETY: `-commit`은 () -> void이며 frame당 한 번만 부릅니다(open을 소비).
            unsafe {
                let _: () = open.command_buffer.send(sel!("commit"), ());
            }
        });
        let id = SubmissionId(self.next_submission);
        self.next_submission += 1;
        self.in_flight.push_back(InFlight {
            id: Some(id),
            command_buffer: open.command_buffer,
            present: open.present,
        });
        Ok(id)
    }

    fn discard(&mut self, frame: EncodedFrame) -> Result<(), BackendError> {
        // commit하지 않은 command buffer와 drawable을 놓습니다. GPU는 이 frame을 보지 않습니다.
        let open = self.take_open(&frame)?;
        autoreleasepool(|| drop(open));
        Ok(())
    }

    fn poll(&mut self) -> Result<GpuProgress, BackendError> {
        self.device()?;
        autoreleasepool(|| self.poll_completions())
    }

    fn query_timestamps(
        &mut self,
        _submission: SubmissionId,
    ) -> Result<TimestampQuery, BackendError> {
        self.device()?;
        Ok(TimestampQuery::Unsupported)
    }
}

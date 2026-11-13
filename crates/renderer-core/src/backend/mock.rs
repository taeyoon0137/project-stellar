//! 테스트용 가짜 backend.
//!
//! 호출을 기록하고, GPU 실행을 테스트가 직접 진행시킵니다([`MockBackend::complete_next`]).
//! 오류 주입([`MockBackend::fail_next`])으로 오류 경로를 재현합니다.

use std::collections::{BTreeSet, VecDeque};

use crate::backend::{
    BackendCall, BackendError, BackendErrorKind, BackendKind, BufferDesc, BufferHandle,
    DeviceCapabilities, DeviceConfig, EncodeOutcome, EncodedFrame, FrameEncodeRequest, FrameId,
    GpuProgress, GpuTextureDesc, GpuTimings, PassGpuTime, RendererBackend, SkipReason,
    SubmissionId, SurfaceSize, TextureHandle, TextureRegion, TimestampQuery,
};
use crate::render_graph::PassId;

/// 제출된 submission 기록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MockSubmission {
    pub id: SubmissionId,
    pub frame: FrameId,
    pub present: bool,
    pub passes: Vec<PassId>,
}

/// 테스트용 가짜 backend.
#[derive(Debug)]
pub(crate) struct MockBackend {
    /// 내부 backend까지 도달한 호출(순서대로).
    pub calls: Vec<BackendCall>,
    /// init이 돌려줄 capability.
    pub caps: DeviceCapabilities,
    /// 다음 해당 호출 하나를 이 종류의 오류로 실패시킵니다.
    pub fail_next: Option<(BackendCall, BackendErrorKind)>,
    /// drawable을 얻지 못하는 상황을 흉내 냅니다.
    pub no_drawable: bool,
    timestamps: bool,
    surface: SurfaceSize,
    next_handle: u64,
    buffers: BTreeSet<u64>,
    textures: BTreeSet<u64>,
    open: Option<(FrameId, Vec<PassId>, bool)>,
    next_submission: u64,
    in_flight: VecDeque<MockSubmission>,
    completed: Vec<MockSubmission>,
    progress: GpuProgress,
}

impl MockBackend {
    pub(crate) fn new() -> MockBackend {
        MockBackend {
            calls: Vec::new(),
            caps: DeviceCapabilities {
                backend: BackendKind::Mock,
                max_texture_dimension: 16_384,
                display_p3_surface: true,
                timestamp_query: true,
                msaa_x4: true,
                stencil_bits: 8,
            },
            fail_next: None,
            no_drawable: false,
            timestamps: false,
            surface: SurfaceSize {
                width: 0,
                height: 0,
            },
            next_handle: 1,
            buffers: BTreeSet::new(),
            textures: BTreeSet::new(),
            open: None,
            next_submission: 1,
            in_flight: VecDeque::new(),
            completed: Vec::new(),
            progress: GpuProgress::default(),
        }
    }

    /// GPU가 가장 오래된 submission을 끝냅니다. present 예약이 있으면 present도 끝납니다.
    pub(crate) fn complete_next(&mut self) -> Option<SubmissionId> {
        let done = self.in_flight.pop_front()?;
        self.progress.completed = Some(done.id);
        if done.present {
            self.progress.presented = Some(done.id);
        }
        let id = done.id;
        self.completed.push(done);
        Some(id)
    }

    /// GPU가 밀린 submission을 모두 끝냅니다.
    pub(crate) fn complete_all(&mut self) {
        while self.complete_next().is_some() {}
    }

    /// GPU에서 아직 끝나지 않은 submission 수.
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// 끝난 submission 기록.
    pub(crate) fn completed(&self) -> &[MockSubmission] {
        &self.completed
    }

    /// 살아 있는 buffer·texture 수.
    pub(crate) fn live_resources(&self) -> (usize, usize) {
        (self.buffers.len(), self.textures.len())
    }

    fn enter(&mut self, call: BackendCall) -> Result<(), BackendError> {
        self.calls.push(call);
        match self.fail_next {
            Some((target, kind)) if target == call => {
                self.fail_next = None;
                Err(BackendError::new(kind, "mock 주입 오류"))
            }
            _ => Ok(()),
        }
    }

    fn issue(&mut self) -> u64 {
        let handle = self.next_handle;
        self.next_handle += 1;
        handle
    }
}

const UNKNOWN_HANDLE: BackendError =
    BackendError::new(BackendErrorKind::InvalidUsage, "mock: 없는 handle");

impl RendererBackend for MockBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Mock
    }

    fn init(&mut self, config: &DeviceConfig) -> Result<DeviceCapabilities, BackendError> {
        self.enter(BackendCall::Init)?;
        self.surface = config.surface.size;
        self.timestamps = config.timestamps && self.caps.timestamp_query;
        // 새 device: 이전 resource와 submission은 모두 사라집니다.
        self.buffers.clear();
        self.textures.clear();
        self.open = None;
        self.in_flight.clear();
        self.next_submission = 1;
        self.progress = GpuProgress::default();
        Ok(self.caps)
    }

    fn resize(&mut self, size: SurfaceSize) -> Result<(), BackendError> {
        self.enter(BackendCall::Resize)?;
        self.surface = size;
        Ok(())
    }

    fn create_buffer(&mut self, desc: &BufferDesc) -> Result<BufferHandle, BackendError> {
        self.enter(BackendCall::CreateBuffer)?;
        if desc.size == 0 {
            return Err(BackendError::new(
                BackendErrorKind::InvalidUsage,
                "mock: 0 크기 buffer",
            ));
        }
        let handle = self.issue();
        self.buffers.insert(handle);
        Ok(BufferHandle::from_raw(handle))
    }

    fn create_texture(&mut self, desc: &GpuTextureDesc) -> Result<TextureHandle, BackendError> {
        self.enter(BackendCall::CreateTexture)?;
        if desc.width == 0 || desc.height == 0 || desc.mip_levels == 0 {
            return Err(BackendError::new(
                BackendErrorKind::InvalidUsage,
                "mock: 잘못된 texture descriptor",
            ));
        }
        let handle = self.issue();
        self.textures.insert(handle);
        Ok(TextureHandle::from_raw(handle))
    }

    fn update_buffer(
        &mut self,
        buffer: BufferHandle,
        _offset: u64,
        _data: &[u8],
    ) -> Result<(), BackendError> {
        self.enter(BackendCall::UpdateBuffer)?;
        if !self.buffers.contains(&buffer.raw()) {
            return Err(UNKNOWN_HANDLE);
        }
        Ok(())
    }

    fn update_texture(
        &mut self,
        texture: TextureHandle,
        _region: TextureRegion,
        _data: &[u8],
    ) -> Result<(), BackendError> {
        self.enter(BackendCall::UpdateTexture)?;
        if !self.textures.contains(&texture.raw()) {
            return Err(UNKNOWN_HANDLE);
        }
        Ok(())
    }

    fn destroy_buffer(&mut self, buffer: BufferHandle) -> Result<(), BackendError> {
        self.enter(BackendCall::DestroyBuffer)?;
        if !self.buffers.remove(&buffer.raw()) {
            return Err(UNKNOWN_HANDLE);
        }
        Ok(())
    }

    fn destroy_texture(&mut self, texture: TextureHandle) -> Result<(), BackendError> {
        self.enter(BackendCall::DestroyTexture)?;
        if !self.textures.remove(&texture.raw()) {
            return Err(UNKNOWN_HANDLE);
        }
        Ok(())
    }

    fn encode(&mut self, request: &FrameEncodeRequest<'_>) -> Result<EncodeOutcome, BackendError> {
        self.enter(BackendCall::Encode)?;
        let surface = request.targets_surface();
        if surface && self.surface.is_empty() {
            return Ok(EncodeOutcome::Skipped(SkipReason::EmptySurface));
        }
        if surface && self.no_drawable {
            return Ok(EncodeOutcome::Skipped(SkipReason::NoDrawable));
        }
        let passes = request.graph.order().to_vec();
        self.open = Some((request.frame, passes, false));
        Ok(EncodeOutcome::Encoded(EncodedFrame::new(
            request.frame,
            request.frame.0,
        )))
    }

    fn schedule_present(&mut self, frame: &EncodedFrame) -> Result<(), BackendError> {
        self.enter(BackendCall::SchedulePresent)?;
        match self.open.as_mut() {
            Some((open, _, present)) if *open == frame.frame() => {
                *present = true;
                Ok(())
            }
            _ => Err(BackendError::new(
                BackendErrorKind::InvalidUsage,
                "mock: 열린 frame 없음",
            )),
        }
    }

    fn submit(&mut self, frame: EncodedFrame) -> Result<SubmissionId, BackendError> {
        let open = self.open.take();
        self.enter(BackendCall::Submit)?;
        let Some((open_frame, passes, present)) = open else {
            return Err(BackendError::new(
                BackendErrorKind::InvalidUsage,
                "mock: 열린 frame 없음",
            ));
        };
        if open_frame != frame.frame() {
            return Err(BackendError::new(
                BackendErrorKind::InvalidUsage,
                "mock: 다른 frame",
            ));
        }
        let id = SubmissionId(self.next_submission);
        self.next_submission += 1;
        self.in_flight.push_back(MockSubmission {
            id,
            frame: open_frame,
            present,
            passes,
        });
        Ok(id)
    }

    fn poll(&mut self) -> Result<GpuProgress, BackendError> {
        self.enter(BackendCall::Poll)?;
        Ok(self.progress)
    }

    fn query_timestamps(
        &mut self,
        submission: SubmissionId,
    ) -> Result<TimestampQuery, BackendError> {
        self.enter(BackendCall::QueryTimestamps)?;
        if !self.timestamps {
            return Ok(TimestampQuery::Unsupported);
        }
        let Some(done) = self.completed.iter().find(|done| done.id == submission) else {
            return Ok(TimestampQuery::Pending);
        };
        // pass마다 1µs씩 걸린 것으로 기록합니다.
        let passes: Vec<PassGpuTime> = done
            .passes
            .iter()
            .map(|&pass| PassGpuTime { pass, nanos: 1_000 })
            .collect();
        let total_nanos = passes.iter().map(|pass| pass.nanos).sum();
        Ok(TimestampQuery::Ready(GpuTimings {
            passes,
            total_nanos,
        }))
    }
}

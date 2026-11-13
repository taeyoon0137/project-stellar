//! backend 호출 순서 검사 (XI.5.1).
//!
//! [`CallOrderValidator`]는 [`RendererBackend`] 호출 순서 계약(모듈 [`crate::backend`] 문서)을
//! 상태 기계로 검사합니다. [`CheckedBackend`]는 backend를 감싸 호출마다 validator를 거치고, 위반이면
//! 내부 backend를 호출하지 않고 [`BackendError::call_order`]를 돌려줍니다.

use std::collections::BTreeSet;
use std::fmt;

use crate::backend::recovery::Recovery;
use crate::backend::{
    BackendError, BackendErrorKind, BackendKind, BufferDesc, BufferHandle, DeviceCapabilities,
    DeviceConfig, EncodeOutcome, EncodedFrame, FrameEncodeRequest, FrameId, GpuProgress,
    GpuTextureDesc, ImportTarget, RendererBackend, SubmissionId, SurfaceSize, TextureHandle,
    TextureRegion, TimestampQuery, MAX_FRAMES_IN_FLIGHT,
};
use crate::render_graph::{ImportKind, PassId, ResourceId, ResourceLifetime};

/// backend 호출 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendCall {
    /// `init`.
    Init,
    /// `resize`.
    Resize,
    /// `create_buffer`.
    CreateBuffer,
    /// `create_texture`.
    CreateTexture,
    /// `update_buffer`.
    UpdateBuffer,
    /// `update_texture`.
    UpdateTexture,
    /// `destroy_buffer`.
    DestroyBuffer,
    /// `destroy_texture`.
    DestroyTexture,
    /// `encode`.
    Encode,
    /// `schedule_present`.
    SchedulePresent,
    /// `submit`.
    Submit,
    /// `poll`.
    Poll,
    /// `query_timestamps`.
    QueryTimestamps,
}

/// validator 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallOrderState {
    /// `init` 전.
    Uninitialized,
    /// frame이 열려 있지 않음.
    Idle,
    /// encode한 frame이 submit을 기다림.
    FrameOpen,
    /// device lost. `init`만 허용.
    Lost,
}

/// 호출 순서·요청 위반.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallOrderError {
    /// `init` 전 호출.
    NotInitialized {
        /// 호출.
        call: BackendCall,
    },
    /// 이미 초기화됨.
    AlreadyInitialized,
    /// device lost 뒤 `init` 아닌 호출.
    DeviceLost {
        /// 호출.
        call: BackendCall,
    },
    /// frame이 열린 동안 허용되지 않는 호출.
    FrameOpen {
        /// 호출.
        call: BackendCall,
    },
    /// 열린 frame 없이 frame 호출.
    NoOpenFrame {
        /// 호출.
        call: BackendCall,
    },
    /// 열린 frame과 다른 frame.
    FrameMismatch {
        /// 열린 frame.
        open: FrameId,
        /// 넘긴 frame.
        given: FrameId,
    },
    /// present를 이미 예약함.
    PresentAlreadyScheduled,
    /// surface에 그리지 않는 frame에 present 예약.
    PresentWithoutSurface,
    /// frame 번호가 증가하지 않음.
    FrameNotIncreasing {
        /// 직전 frame.
        previous: FrameId,
        /// 요청 frame.
        frame: FrameId,
    },
    /// backend가 증가하지 않는 submission 번호를 발급함.
    SubmissionNotIncreasing {
        /// 직전 submission.
        previous: SubmissionId,
        /// 받은 submission.
        submission: SubmissionId,
    },
    /// 아직 없는 submission의 timestamp 조회.
    UnknownSubmission {
        /// submission.
        submission: SubmissionId,
    },
    /// frames in flight가 1..=3이 아님.
    InvalidFramesInFlight {
        /// 요청 값.
        requested: u8,
    },
    /// encode 요청의 pass command가 graph에 없는 pass를 가리킴.
    UnknownPass {
        /// pass.
        pass: PassId,
    },
    /// 같은 pass의 command 목록이 둘 이상.
    DuplicatePassCommands {
        /// pass.
        pass: PassId,
    },
    /// imported resource에 binding이 없음.
    MissingImport {
        /// resource.
        resource: ResourceId,
    },
    /// binding이 imported resource가 아니거나 import 종류와 대상이 맞지 않거나 중복됨.
    InvalidImport {
        /// resource.
        resource: ResourceId,
    },
}

impl fmt::Display for CallOrderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallOrderError::NotInitialized { call } => write!(f, "init 전 {call:?} 호출"),
            CallOrderError::AlreadyInitialized => f.write_str("이미 초기화된 backend에 init"),
            CallOrderError::DeviceLost { call } => write!(f, "device lost 뒤 {call:?} 호출"),
            CallOrderError::FrameOpen { call } => write!(f, "frame이 열린 동안 {call:?} 호출"),
            CallOrderError::NoOpenFrame { call } => write!(f, "열린 frame 없이 {call:?} 호출"),
            CallOrderError::FrameMismatch { open, given } => {
                write!(f, "열린 frame {}과 다른 frame {}", open.0, given.0)
            }
            CallOrderError::PresentAlreadyScheduled => f.write_str("present를 이미 예약했습니다"),
            CallOrderError::PresentWithoutSurface => {
                f.write_str("surface에 그리지 않는 frame은 present할 수 없습니다")
            }
            CallOrderError::FrameNotIncreasing { previous, frame } => {
                write!(
                    f,
                    "frame 번호가 증가하지 않습니다: {} → {}",
                    previous.0, frame.0
                )
            }
            CallOrderError::SubmissionNotIncreasing {
                previous,
                submission,
            } => write!(
                f,
                "submission 번호가 증가하지 않습니다: {} → {}",
                previous.0, submission.0
            ),
            CallOrderError::UnknownSubmission { submission } => {
                write!(f, "아직 없는 submission {}", submission.0)
            }
            CallOrderError::InvalidFramesInFlight { requested } => write!(
                f,
                "frames in flight는 1..={MAX_FRAMES_IN_FLIGHT}이어야 합니다: {requested}"
            ),
            CallOrderError::UnknownPass { pass } => write!(f, "graph에 없는 pass {}", pass.0),
            CallOrderError::DuplicatePassCommands { pass } => {
                write!(f, "pass {}의 command 목록이 중복됩니다", pass.0)
            }
            CallOrderError::MissingImport { resource } => {
                write!(f, "imported resource {}의 binding이 없습니다", resource.0)
            }
            CallOrderError::InvalidImport { resource } => {
                write!(
                    f,
                    "resource {}의 import binding이 잘못되었습니다",
                    resource.0
                )
            }
        }
    }
}

impl std::error::Error for CallOrderError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenFrame {
    frame: FrameId,
    targets_surface: bool,
    present_scheduled: bool,
}

/// 호출 순서 상태 기계.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallOrderValidator {
    state: CallOrderState,
    open: Option<OpenFrame>,
    last_frame: Option<FrameId>,
    last_submission: Option<SubmissionId>,
}

impl Default for CallOrderValidator {
    fn default() -> CallOrderValidator {
        CallOrderValidator::new()
    }
}

impl CallOrderValidator {
    /// `init` 전 상태.
    pub const fn new() -> CallOrderValidator {
        CallOrderValidator {
            state: CallOrderState::Uninitialized,
            open: None,
            last_frame: None,
            last_submission: None,
        }
    }

    /// 현재 상태.
    pub const fn state(&self) -> CallOrderState {
        self.state
    }

    /// 마지막으로 제출된 submission.
    pub const fn last_submission(&self) -> Option<SubmissionId> {
        self.last_submission
    }

    /// 상태만으로 판단하는 호출 허용 검사.
    pub fn check(&self, call: BackendCall) -> Result<(), CallOrderError> {
        match (self.state, call) {
            (CallOrderState::Uninitialized | CallOrderState::Lost, BackendCall::Init) => Ok(()),
            (_, BackendCall::Init) => Err(CallOrderError::AlreadyInitialized),
            (CallOrderState::Uninitialized, call) => Err(CallOrderError::NotInitialized { call }),
            (CallOrderState::Lost, call) => Err(CallOrderError::DeviceLost { call }),
            (_, BackendCall::Poll | BackendCall::QueryTimestamps) => Ok(()),
            (CallOrderState::FrameOpen, BackendCall::SchedulePresent | BackendCall::Submit) => {
                Ok(())
            }
            (CallOrderState::Idle, BackendCall::SchedulePresent | BackendCall::Submit) => {
                Err(CallOrderError::NoOpenFrame { call })
            }
            (CallOrderState::FrameOpen, call) => Err(CallOrderError::FrameOpen { call }),
            (CallOrderState::Idle, _) => Ok(()),
        }
    }

    /// `init` 호출 전 검사.
    pub fn before_init(&self, config: &DeviceConfig) -> Result<(), CallOrderError> {
        self.check(BackendCall::Init)?;
        if config.frames_in_flight == 0 || config.frames_in_flight > MAX_FRAMES_IN_FLIGHT {
            return Err(CallOrderError::InvalidFramesInFlight {
                requested: config.frames_in_flight,
            });
        }
        Ok(())
    }

    /// `init` 성공 기록. 새 device의 submission 번호는 처음부터 다시 셉니다.
    pub fn after_init(&mut self) {
        self.state = CallOrderState::Idle;
        self.open = None;
        self.last_submission = None;
    }

    /// `encode` 호출 전 검사(frame 번호 증가, pass·import 구성).
    pub fn before_encode(&self, request: &FrameEncodeRequest<'_>) -> Result<(), CallOrderError> {
        self.check(BackendCall::Encode)?;
        if let Some(previous) = self.last_frame {
            if request.frame <= previous {
                return Err(CallOrderError::FrameNotIncreasing {
                    previous,
                    frame: request.frame,
                });
            }
        }
        let graph = request.graph.graph();
        let mut seen: BTreeSet<PassId> = BTreeSet::new();
        for commands in request.passes {
            if request.graph.pass(commands.pass).is_none() {
                return Err(CallOrderError::UnknownPass {
                    pass: commands.pass,
                });
            }
            if !seen.insert(commands.pass) {
                return Err(CallOrderError::DuplicatePassCommands {
                    pass: commands.pass,
                });
            }
        }
        let mut bound: BTreeSet<ResourceId> = BTreeSet::new();
        for binding in request.imports {
            let kind = match request.graph.resource(binding.resource).map(|r| r.lifetime) {
                Some(ResourceLifetime::Imported(kind)) => kind,
                _ => {
                    return Err(CallOrderError::InvalidImport {
                        resource: binding.resource,
                    })
                }
            };
            let matches = matches!(
                (kind, binding.target),
                (ImportKind::Surface, ImportTarget::Surface)
                    | (ImportKind::External, ImportTarget::Texture(_))
            );
            if !matches || !bound.insert(binding.resource) {
                return Err(CallOrderError::InvalidImport {
                    resource: binding.resource,
                });
            }
        }
        for (index, resource) in graph.resources.iter().enumerate() {
            let id = ResourceId(index as u32);
            if matches!(resource.lifetime, ResourceLifetime::Imported(_)) && !bound.contains(&id) {
                return Err(CallOrderError::MissingImport { resource: id });
            }
        }
        Ok(())
    }

    /// `encode` 결과 기록. 성공이든 skip이든 frame 번호는 소비됩니다.
    pub fn after_encode(&mut self, request: &FrameEncodeRequest<'_>, outcome: &EncodeOutcome) {
        self.last_frame = Some(request.frame);
        if let EncodeOutcome::Encoded(_) = outcome {
            self.state = CallOrderState::FrameOpen;
            self.open = Some(OpenFrame {
                frame: request.frame,
                targets_surface: request.targets_surface(),
                present_scheduled: false,
            });
        }
    }

    fn open_frame(&self, call: BackendCall, frame: FrameId) -> Result<OpenFrame, CallOrderError> {
        self.check(call)?;
        let open = self.open.ok_or(CallOrderError::NoOpenFrame { call })?;
        if open.frame != frame {
            return Err(CallOrderError::FrameMismatch {
                open: open.frame,
                given: frame,
            });
        }
        Ok(open)
    }

    /// `schedule_present` 호출 전 검사.
    pub fn before_present(&self, frame: &EncodedFrame) -> Result<(), CallOrderError> {
        let open = self.open_frame(BackendCall::SchedulePresent, frame.frame())?;
        if !open.targets_surface {
            return Err(CallOrderError::PresentWithoutSurface);
        }
        if open.present_scheduled {
            return Err(CallOrderError::PresentAlreadyScheduled);
        }
        Ok(())
    }

    /// `schedule_present` 성공 기록.
    pub fn after_present(&mut self) {
        if let Some(open) = self.open.as_mut() {
            open.present_scheduled = true;
        }
    }

    /// 열린 frame에 present가 예약됐으면 참입니다.
    pub fn present_scheduled(&self) -> bool {
        self.open.is_some_and(|open| open.present_scheduled)
    }

    /// `submit` 호출 전 검사.
    pub fn before_submit(&self, frame: &EncodedFrame) -> Result<(), CallOrderError> {
        self.open_frame(BackendCall::Submit, frame.frame())
            .map(|_| ())
    }

    /// `submit` 결과 기록. frame은 성공·실패와 무관하게 닫힙니다.
    pub fn after_submit(
        &mut self,
        result: Result<SubmissionId, BackendErrorKind>,
    ) -> Result<(), CallOrderError> {
        self.open = None;
        if self.state == CallOrderState::FrameOpen {
            self.state = CallOrderState::Idle;
        }
        let Ok(submission) = result else {
            return Ok(());
        };
        if let Some(previous) = self.last_submission {
            if submission <= previous {
                return Err(CallOrderError::SubmissionNotIncreasing {
                    previous,
                    submission,
                });
            }
        }
        self.last_submission = Some(submission);
        Ok(())
    }

    /// `query_timestamps` 호출 전 검사.
    pub fn before_timestamps(&self, submission: SubmissionId) -> Result<(), CallOrderError> {
        self.check(BackendCall::QueryTimestamps)?;
        match self.last_submission {
            Some(last) if submission <= last => Ok(()),
            _ => Err(CallOrderError::UnknownSubmission { submission }),
        }
    }

    /// 호출 결과가 device 재생성이 필요한 오류(`DeviceLost`, `Internal`, XI.5.4)면 상태를 `Lost`로
    /// 바꿉니다. 이후 `init`만 허용합니다.
    pub fn observe_error(&mut self, error: &BackendError) {
        if error.recovery() == Recovery::RecreateDevice {
            self.state = CallOrderState::Lost;
            self.open = None;
        }
    }
}

/// 호출 순서를 검사하는 backend wrapper.
#[derive(Debug)]
pub struct CheckedBackend<B> {
    inner: B,
    validator: CallOrderValidator,
}

impl<B: RendererBackend> CheckedBackend<B> {
    /// backend를 감쌉니다.
    pub fn new(inner: B) -> CheckedBackend<B> {
        CheckedBackend {
            inner,
            validator: CallOrderValidator::new(),
        }
    }

    /// 내부 backend.
    pub fn inner(&self) -> &B {
        &self.inner
    }

    /// 내부 backend(가변). 호출 순서 검사를 우회하므로 진단·테스트 전용으로 씁니다.
    pub fn inner_mut(&mut self) -> &mut B {
        &mut self.inner
    }

    /// validator.
    pub fn validator(&self) -> &CallOrderValidator {
        &self.validator
    }

    /// 내부 backend를 돌려줍니다.
    pub fn into_inner(self) -> B {
        self.inner
    }

    fn observe<T>(&mut self, result: Result<T, BackendError>) -> Result<T, BackendError> {
        if let Err(error) = &result {
            self.validator.observe_error(error);
        }
        result
    }

    fn gate(&self, call: BackendCall) -> Result<(), BackendError> {
        self.validator.check(call).map_err(BackendError::call_order)
    }
}

impl<B: RendererBackend> RendererBackend for CheckedBackend<B> {
    fn kind(&self) -> BackendKind {
        self.inner.kind()
    }

    fn init(&mut self, config: &DeviceConfig) -> Result<DeviceCapabilities, BackendError> {
        self.validator
            .before_init(config)
            .map_err(BackendError::call_order)?;
        let result = self.inner.init(config);
        if result.is_ok() {
            self.validator.after_init();
        }
        self.observe(result)
    }

    fn resize(&mut self, size: SurfaceSize) -> Result<(), BackendError> {
        self.gate(BackendCall::Resize)?;
        let result = self.inner.resize(size);
        self.observe(result)
    }

    fn create_buffer(&mut self, desc: &BufferDesc) -> Result<BufferHandle, BackendError> {
        self.gate(BackendCall::CreateBuffer)?;
        let result = self.inner.create_buffer(desc);
        self.observe(result)
    }

    fn create_texture(&mut self, desc: &GpuTextureDesc) -> Result<TextureHandle, BackendError> {
        self.gate(BackendCall::CreateTexture)?;
        let result = self.inner.create_texture(desc);
        self.observe(result)
    }

    fn update_buffer(
        &mut self,
        buffer: BufferHandle,
        offset: u64,
        data: &[u8],
    ) -> Result<(), BackendError> {
        self.gate(BackendCall::UpdateBuffer)?;
        let result = self.inner.update_buffer(buffer, offset, data);
        self.observe(result)
    }

    fn update_texture(
        &mut self,
        texture: TextureHandle,
        region: TextureRegion,
        data: &[u8],
    ) -> Result<(), BackendError> {
        self.gate(BackendCall::UpdateTexture)?;
        let result = self.inner.update_texture(texture, region, data);
        self.observe(result)
    }

    fn destroy_buffer(&mut self, buffer: BufferHandle) -> Result<(), BackendError> {
        self.gate(BackendCall::DestroyBuffer)?;
        let result = self.inner.destroy_buffer(buffer);
        self.observe(result)
    }

    fn destroy_texture(&mut self, texture: TextureHandle) -> Result<(), BackendError> {
        self.gate(BackendCall::DestroyTexture)?;
        let result = self.inner.destroy_texture(texture);
        self.observe(result)
    }

    fn encode(&mut self, request: &FrameEncodeRequest<'_>) -> Result<EncodeOutcome, BackendError> {
        self.validator
            .before_encode(request)
            .map_err(BackendError::call_order)?;
        let result = self.inner.encode(request);
        if let Ok(outcome) = &result {
            self.validator.after_encode(request, outcome);
        }
        self.observe(result)
    }

    fn schedule_present(&mut self, frame: &EncodedFrame) -> Result<(), BackendError> {
        self.validator
            .before_present(frame)
            .map_err(BackendError::call_order)?;
        let result = self.inner.schedule_present(frame);
        if result.is_ok() {
            self.validator.after_present();
        }
        self.observe(result)
    }

    fn submit(&mut self, frame: EncodedFrame) -> Result<SubmissionId, BackendError> {
        self.validator
            .before_submit(&frame)
            .map_err(BackendError::call_order)?;
        let result = self.inner.submit(frame);
        self.validator
            .after_submit(result.as_ref().map(|id| *id).map_err(|e| e.kind()))
            .map_err(BackendError::call_order)?;
        self.observe(result)
    }

    fn poll(&mut self) -> Result<GpuProgress, BackendError> {
        self.gate(BackendCall::Poll)?;
        let result = self.inner.poll();
        self.observe(result)
    }

    fn query_timestamps(
        &mut self,
        submission: SubmissionId,
    ) -> Result<TimestampQuery, BackendError> {
        self.validator
            .before_timestamps(submission)
            .map_err(BackendError::call_order)?;
        let result = self.inner.query_timestamps(submission);
        self.observe(result)
    }
}

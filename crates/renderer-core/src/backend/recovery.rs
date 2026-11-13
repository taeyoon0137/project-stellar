//! backend 오류 분류와 복구 정책 (XI.5.4).
//!
//! [`BackendErrorKind`]마다 **기본 복구 방법**([`Recovery`])과 복구 가능 여부, 안정 숫자 코드를
//! 고정합니다. [`RecoveryTracker`]는 같은 오류가 연달아 날 때 복구 단계를 올려(escalation) 무한 재시도
//! loop를 막습니다. 어떤 오류도 앱 crash로 이어지지 않으며, 복구 불가 오류는 renderer를 비활성화하고
//! 오류 event로 보고합니다(아키텍처 §39: 렌더러 오류 코드만 진단에 남김).
//!
//! | 종류 | 기본 복구 | 복구 가능 | 연속 발생 시 |
//! |---|---|---|---|
//! | `Timeout` | 다음 frame 재시도 | 예 | 3회 연속이면 surface 재구성 |
//! | `SurfaceLost` | surface 재구성 | 예 | 2회 연속이면 device 재생성 |
//! | `OutOfDeviceMemory`·`OutOfHostMemory` | cache purge 후 **한 번만** 재시도 | 예 | 재시도도 실패하면 frame skip |
//! | `DeviceLost`·`Internal` | device 재생성 | 예 | 3회 연속이면 비활성화 |
//! | `InvalidUsage` | frame skip + 보고 | 예 | (재시도 무의미, 단계 없음) |
//! | `Unsupported`·`PipelineCreation` | 비활성화 + 보고 | 아니요 | — |
//!
//! "연속"은 그 사이 성공한 frame([`RecoveryTracker::frame_succeeded`])이 없다는 뜻입니다.

use crate::backend::{BackendError, BackendErrorKind};

/// 오류 종류별 기본 복구 방법.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Recovery {
    /// 다음 frame에서 그대로 다시 시도합니다(일시적 오류).
    RetryNextFrame,
    /// surface를 다시 구성(`resize`)하고 계속합니다.
    ReconfigureSurface,
    /// 재생성 가능한 cache(tessellation·mipmap·glyph page)를 비우고 실패한 작업을 한 번만 다시 합니다.
    PurgeCachesAndRetryOnce,
    /// device를 다시 만들고(`init`) 모든 GPU resource를 다시 올립니다.
    RecreateDevice,
    /// 이 frame을 버리고 진단을 남깁니다(공통 계층 버그, 재시도 무의미).
    SkipFrameAndReport,
    /// renderer를 비활성화하고 오류 event를 냅니다(앱은 계속 동작).
    Disable,
}

/// 연속 timeout 몇 번째에 surface를 재구성할지.
pub const TIMEOUT_ESCALATION: u32 = 3;

/// 연속 surface lost 몇 번째에 device를 재생성할지.
pub const SURFACE_LOST_ESCALATION: u32 = 2;

/// 연속 device 재생성 시도 상한. 넘으면 비활성화합니다.
pub const MAX_DEVICE_RECREATIONS: u32 = 3;

impl BackendErrorKind {
    /// 정의된 모든 종류(안정 순서).
    pub const ALL: [BackendErrorKind; 9] = [
        BackendErrorKind::InvalidUsage,
        BackendErrorKind::Unsupported,
        BackendErrorKind::DeviceLost,
        BackendErrorKind::OutOfDeviceMemory,
        BackendErrorKind::OutOfHostMemory,
        BackendErrorKind::SurfaceLost,
        BackendErrorKind::Timeout,
        BackendErrorKind::PipelineCreation,
        BackendErrorKind::Internal,
    ];

    /// 기본 복구 방법(모듈 문서 표).
    pub const fn recovery(self) -> Recovery {
        match self {
            BackendErrorKind::Timeout => Recovery::RetryNextFrame,
            BackendErrorKind::SurfaceLost => Recovery::ReconfigureSurface,
            BackendErrorKind::OutOfDeviceMemory | BackendErrorKind::OutOfHostMemory => {
                Recovery::PurgeCachesAndRetryOnce
            }
            BackendErrorKind::DeviceLost | BackendErrorKind::Internal => Recovery::RecreateDevice,
            BackendErrorKind::InvalidUsage => Recovery::SkipFrameAndReport,
            BackendErrorKind::Unsupported | BackendErrorKind::PipelineCreation => Recovery::Disable,
        }
    }

    /// 자동 복구로 렌더링을 이어 갈 수 있으면 참입니다.
    pub const fn is_recoverable(self) -> bool {
        !matches!(self.recovery(), Recovery::Disable)
    }

    /// 진단·telemetry용 안정 렌더러 오류 코드(값을 바꾸거나 재사용하지 않음).
    pub const fn code(self) -> u32 {
        match self {
            BackendErrorKind::InvalidUsage => 0x0B01,
            BackendErrorKind::Unsupported => 0x0B02,
            BackendErrorKind::DeviceLost => 0x0B03,
            BackendErrorKind::OutOfDeviceMemory => 0x0B04,
            BackendErrorKind::OutOfHostMemory => 0x0B05,
            BackendErrorKind::SurfaceLost => 0x0B06,
            BackendErrorKind::Timeout => 0x0B07,
            BackendErrorKind::PipelineCreation => 0x0B08,
            BackendErrorKind::Internal => 0x0B09,
        }
    }

    /// 안정 코드에서 종류를 찾습니다.
    pub fn from_code(code: u32) -> Option<BackendErrorKind> {
        BackendErrorKind::ALL
            .into_iter()
            .find(|kind| kind.code() == code)
    }

    /// 구조화 오류(II.3.1 `ErrorCode`)의 숫자 분류. 렌더러 오류는 사용자 입력 문제가 아니므로 모두
    /// `ErrorCode::Internal`(1)이며, 세부 종류는 [`BackendErrorKind::code`]를 개발자 context에
    /// 담습니다.
    pub const fn core_error_code(self) -> u32 {
        1
    }

    /// 사용자에게 보여 줄 메시지(내용·경로 없음).
    pub const fn user_message(self) -> &'static str {
        match self {
            BackendErrorKind::DeviceLost | BackendErrorKind::Internal => {
                "그래픽 장치를 다시 연결하는 중입니다."
            }
            BackendErrorKind::OutOfDeviceMemory | BackendErrorKind::OutOfHostMemory => {
                "메모리가 부족해 일부 화면을 다시 그리는 중입니다."
            }
            BackendErrorKind::SurfaceLost | BackendErrorKind::Timeout => {
                "화면을 다시 그리는 중입니다."
            }
            BackendErrorKind::InvalidUsage => "화면 갱신 중 오류가 발생했습니다.",
            BackendErrorKind::Unsupported | BackendErrorKind::PipelineCreation => {
                "이 그래픽 장치에서는 캔버스를 표시할 수 없습니다."
            }
        }
    }
}

impl BackendError {
    /// 기본 복구 방법.
    pub const fn recovery(&self) -> Recovery {
        self.kind().recovery()
    }

    /// 자동 복구 가능 여부.
    pub const fn is_recoverable(&self) -> bool {
        self.kind().is_recoverable()
    }
}

/// tracker가 결정한 이번 오류의 조치.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryAction {
    /// 아무것도 바꾸지 않고 다음 frame을 그립니다.
    RetryNextFrame,
    /// `resize`로 surface를 재구성한 뒤 다음 frame을 그립니다.
    ReconfigureSurface,
    /// cache를 비우고 실패한 작업을 지금 한 번 더 합니다.
    PurgeCachesAndRetry,
    /// 이 frame을 버립니다(다음 frame은 정상 진행).
    SkipFrame,
    /// `init`으로 device를 재생성하고 resource를 다시 올립니다.
    RecreateDevice,
    /// renderer를 비활성화합니다.
    Disable,
}

/// 연속 오류를 세어 복구 단계를 올리는 tracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RecoveryTracker {
    timeouts: u32,
    surface_losses: u32,
    device_recreations: u32,
    memory_retry_pending: bool,
    disabled: bool,
}

impl RecoveryTracker {
    /// 오류 이력이 없는 tracker.
    pub const fn new() -> RecoveryTracker {
        RecoveryTracker {
            timeouts: 0,
            surface_losses: 0,
            device_recreations: 0,
            memory_retry_pending: false,
            disabled: false,
        }
    }

    /// renderer가 비활성화되었으면 참입니다.
    pub const fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// 오류 하나에 대한 조치를 정합니다.
    pub fn on_error(&mut self, error: &BackendError) -> RecoveryAction {
        if self.disabled {
            return RecoveryAction::Disable;
        }
        let action = match error.recovery() {
            Recovery::RetryNextFrame => {
                self.timeouts = self.timeouts.saturating_add(1);
                if self.timeouts >= TIMEOUT_ESCALATION {
                    self.timeouts = 0;
                    self.surface_losses = self.surface_losses.saturating_add(1);
                    RecoveryAction::ReconfigureSurface
                } else {
                    RecoveryAction::RetryNextFrame
                }
            }
            Recovery::ReconfigureSurface => {
                self.surface_losses = self.surface_losses.saturating_add(1);
                if self.surface_losses >= SURFACE_LOST_ESCALATION {
                    self.surface_losses = 0;
                    self.recreate_device()
                } else {
                    RecoveryAction::ReconfigureSurface
                }
            }
            Recovery::PurgeCachesAndRetryOnce => {
                if self.memory_retry_pending {
                    // purge 후 재시도도 실패: 반복하지 않고 이 frame을 버립니다(XVI.5.6).
                    self.memory_retry_pending = false;
                    RecoveryAction::SkipFrame
                } else {
                    self.memory_retry_pending = true;
                    RecoveryAction::PurgeCachesAndRetry
                }
            }
            Recovery::RecreateDevice => self.recreate_device(),
            Recovery::SkipFrameAndReport => RecoveryAction::SkipFrame,
            Recovery::Disable => RecoveryAction::Disable,
        };
        if action == RecoveryAction::Disable {
            self.disabled = true;
        }
        action
    }

    fn recreate_device(&mut self) -> RecoveryAction {
        self.device_recreations = self.device_recreations.saturating_add(1);
        self.memory_retry_pending = false;
        if self.device_recreations > MAX_DEVICE_RECREATIONS {
            RecoveryAction::Disable
        } else {
            RecoveryAction::RecreateDevice
        }
    }

    /// 실패했던 작업(메모리 부족 재시도 포함)이 성공했습니다.
    pub fn operation_succeeded(&mut self) {
        self.memory_retry_pending = false;
    }

    /// frame 하나를 끝까지 제출했습니다. 연속 오류 계수를 초기화합니다.
    pub fn frame_succeeded(&mut self) {
        if !self.disabled {
            *self = RecoveryTracker::new();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use crate::backend::tests::{device_config, surface_graph};
    use crate::backend::{
        BackendCall, BufferDesc, BufferUsage, CallOrderState, CheckedBackend, EncodeOutcome,
        FrameEncodeRequest, FrameId, GpuTextureDesc, ImportBinding, ImportTarget, RendererBackend,
        SubmissionId, SurfaceSize, TextureRegion, TextureUsage,
    };
    use crate::render_graph::{CompiledRenderGraph, ResourceId, SampleCount, TextureFormat};

    const IMPORTS: [ImportBinding; 1] = [ImportBinding {
        resource: ResourceId(0),
        target: ImportTarget::Surface,
    }];

    fn texture_desc() -> GpuTextureDesc {
        GpuTextureDesc {
            width: 2,
            height: 2,
            format: TextureFormat::Rgba8Unorm,
            mip_levels: 1,
            samples: SampleCount::X1,
            usage: TextureUsage {
                sampled: true,
                render_target: false,
            },
        }
    }

    /// 테스트 대상 backend와 frame 번호.
    struct Harness {
        backend: CheckedBackend<MockBackend>,
        graph: CompiledRenderGraph,
        frame: u64,
    }

    impl Harness {
        fn new() -> Harness {
            Harness {
                backend: CheckedBackend::new(MockBackend::new()),
                graph: surface_graph(),
                frame: 0,
            }
        }

        fn next_request(&mut self) -> FrameId {
            self.frame += 1;
            FrameId(self.frame)
        }

        /// frame 하나를 끝까지(encode → present → submit) 그립니다.
        fn render(&mut self) -> Result<SubmissionId, BackendError> {
            let frame = self.next_request();
            let request = FrameEncodeRequest {
                frame,
                graph: &self.graph,
                passes: &[],
                imports: &IMPORTS,
            };
            let EncodeOutcome::Encoded(encoded) = self.backend.encode(&request)? else {
                panic!("8×8 surface는 encode되어야 합니다");
            };
            self.backend.schedule_present(&encoded)?;
            self.backend.submit(encoded)
        }

        /// 대상 호출이 유효한 상태까지 준비하고, 오류를 주입해 그 호출을 실행합니다.
        fn fail(&mut self, call: BackendCall, kind: BackendErrorKind) -> BackendError {
            if call != BackendCall::Init {
                self.backend.init(&device_config(8, 8)).unwrap();
            }
            let texture = (call != BackendCall::Init)
                .then(|| self.backend.create_texture(&texture_desc()).unwrap());
            let buffer = (call != BackendCall::Init).then(|| {
                self.backend
                    .create_buffer(&BufferDesc {
                        size: 16,
                        usage: BufferUsage::Uniform,
                    })
                    .unwrap()
            });
            let submitted = (call == BackendCall::QueryTimestamps).then(|| self.render().unwrap());
            let region = TextureRegion {
                mip_level: 0,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            };
            let mut open = None;
            if matches!(
                call,
                BackendCall::SchedulePresent | BackendCall::Submit | BackendCall::Discard
            ) {
                let frame = self.next_request();
                let request = FrameEncodeRequest {
                    frame,
                    graph: &self.graph,
                    passes: &[],
                    imports: &IMPORTS,
                };
                let EncodeOutcome::Encoded(encoded) = self.backend.encode(&request).unwrap() else {
                    panic!("encode되어야 합니다");
                };
                open = Some(encoded);
            }
            self.backend.inner_mut().fail_next = Some((call, kind));
            let frame = self.next_request();
            let graph = &self.graph;
            let backend = &mut self.backend;
            let result: Result<(), BackendError> = match call {
                BackendCall::Init => backend.init(&device_config(8, 8)).map(|_| ()),
                BackendCall::Resize => backend.resize(SurfaceSize {
                    width: 4,
                    height: 4,
                }),
                BackendCall::CreateBuffer => backend
                    .create_buffer(&BufferDesc {
                        size: 8,
                        usage: BufferUsage::Vertex,
                    })
                    .map(|_| ()),
                BackendCall::CreateTexture => backend.create_texture(&texture_desc()).map(|_| ()),
                BackendCall::UpdateBuffer => backend.update_buffer(buffer.unwrap(), 0, &[0; 4]),
                BackendCall::UpdateTexture => {
                    backend.update_texture(texture.unwrap(), region, &[0; 4])
                }
                BackendCall::DestroyBuffer => backend.destroy_buffer(buffer.unwrap()),
                BackendCall::DestroyTexture => backend.destroy_texture(texture.unwrap()),
                BackendCall::Encode => backend
                    .encode(&FrameEncodeRequest {
                        frame,
                        graph,
                        passes: &[],
                        imports: &IMPORTS,
                    })
                    .map(|_| ()),
                BackendCall::SchedulePresent => {
                    let encoded = open.take().unwrap();
                    let result = backend.schedule_present(&encoded);
                    // 실패해도 frame은 열려 있으므로 닫아 둡니다(present 없이 제출).
                    if backend.validator().state() == CallOrderState::FrameOpen {
                        backend.submit(encoded).unwrap();
                    }
                    result
                }
                BackendCall::Submit => backend.submit(open.take().unwrap()).map(|_| ()),
                BackendCall::Discard => backend.discard(open.take().unwrap()),
                BackendCall::Poll => backend.poll().map(|_| ()),
                BackendCall::QueryTimestamps => {
                    backend.query_timestamps(submitted.unwrap()).map(|_| ())
                }
            };
            result.unwrap_err()
        }

        /// tracker 조치를 실제로 수행하고, 복구 가능하면 다음 frame이 성공하는지 확인합니다.
        fn recover(
            &mut self,
            tracker: &mut RecoveryTracker,
            call: BackendCall,
            error: &BackendError,
        ) -> RecoveryAction {
            let action = tracker.on_error(error);
            if action == RecoveryAction::Disable {
                return action;
            }
            // init이 실패해 device가 없으면 어떤 조치든 먼저 init을 다시 합니다(재시도에 해당).
            let device_missing = matches!(
                self.backend.validator().state(),
                CallOrderState::Uninitialized | CallOrderState::Lost
            );
            if device_missing {
                assert!(
                    call == BackendCall::Init || action == RecoveryAction::RecreateDevice,
                    "{call:?}: device가 없는데 조치가 {action:?}"
                );
                self.backend.init(&device_config(8, 8)).unwrap();
            }
            match action {
                RecoveryAction::RetryNextFrame | RecoveryAction::SkipFrame => {}
                RecoveryAction::ReconfigureSurface => {
                    self.backend
                        .resize(SurfaceSize {
                            width: 8,
                            height: 8,
                        })
                        .unwrap();
                }
                RecoveryAction::PurgeCachesAndRetry => {
                    // cache purge 뒤 같은 작업을 한 번 다시 합니다(주입 오류는 이미 소비됨).
                    tracker.operation_succeeded();
                }
                RecoveryAction::RecreateDevice => {
                    // device 재생성 오류는 validator를 Lost로 만들므로 위에서 이미 init했습니다.
                    assert!(
                        device_missing,
                        "{call:?}: 재생성 조치인데 device가 살아 있음"
                    );
                }
                RecoveryAction::Disable => return action,
            }
            self.render().unwrap();
            tracker.frame_succeeded();
            action
        }
    }

    #[test]
    fn mock_backend_error_matrix() {
        // 검증 기준(XI.5.4): mock backend error matrix 통과.
        let mut cases = 0;
        for call in BackendCall::ALL {
            for kind in BackendErrorKind::ALL {
                let mut harness = Harness::new();
                let error = harness.fail(call, kind);
                // 1. 오류 종류가 그대로 전달됩니다.
                assert_eq!(error.kind(), kind, "{call:?} {kind:?}");
                assert!(error.call_order_violation().is_none());
                // 2. device 재생성 오류(DeviceLost·Internal)만 validator를 Lost로 만들고, 어떤
                //    오류 뒤에도 frame이 열린 채 남지 않습니다.
                let state = harness.backend.validator().state();
                if kind.recovery() == Recovery::RecreateDevice {
                    assert_eq!(state, CallOrderState::Lost, "{call:?} {kind:?}");
                } else {
                    assert_ne!(state, CallOrderState::Lost, "{call:?} {kind:?}");
                    assert_ne!(state, CallOrderState::FrameOpen, "{call:?} {kind:?}");
                }
                // 3. 분류된 조치를 수행하면 복구 가능한 오류는 다음 frame을 그릴 수 있습니다.
                let mut tracker = RecoveryTracker::new();
                let action = harness.recover(&mut tracker, call, &error);
                assert_eq!(action == RecoveryAction::Disable, !kind.is_recoverable());
                assert_eq!(tracker.is_disabled(), !kind.is_recoverable());
                let expected = match kind.recovery() {
                    Recovery::RetryNextFrame => RecoveryAction::RetryNextFrame,
                    Recovery::ReconfigureSurface => RecoveryAction::ReconfigureSurface,
                    Recovery::PurgeCachesAndRetryOnce => RecoveryAction::PurgeCachesAndRetry,
                    Recovery::RecreateDevice => RecoveryAction::RecreateDevice,
                    Recovery::SkipFrameAndReport => RecoveryAction::SkipFrame,
                    Recovery::Disable => RecoveryAction::Disable,
                };
                assert_eq!(action, expected, "{call:?} {kind:?}");
                cases += 1;
            }
        }
        assert_eq!(cases, BackendCall::ALL.len() * BackendErrorKind::ALL.len());
        assert_eq!(cases, 14 * 9);
    }

    #[test]
    fn classification_and_codes_are_stable() {
        // golden: 코드 값은 진단·telemetry 계약입니다.
        let golden: [(BackendErrorKind, u32, Recovery, bool); 9] = [
            (
                BackendErrorKind::InvalidUsage,
                0x0B01,
                Recovery::SkipFrameAndReport,
                true,
            ),
            (
                BackendErrorKind::Unsupported,
                0x0B02,
                Recovery::Disable,
                false,
            ),
            (
                BackendErrorKind::DeviceLost,
                0x0B03,
                Recovery::RecreateDevice,
                true,
            ),
            (
                BackendErrorKind::OutOfDeviceMemory,
                0x0B04,
                Recovery::PurgeCachesAndRetryOnce,
                true,
            ),
            (
                BackendErrorKind::OutOfHostMemory,
                0x0B05,
                Recovery::PurgeCachesAndRetryOnce,
                true,
            ),
            (
                BackendErrorKind::SurfaceLost,
                0x0B06,
                Recovery::ReconfigureSurface,
                true,
            ),
            (
                BackendErrorKind::Timeout,
                0x0B07,
                Recovery::RetryNextFrame,
                true,
            ),
            (
                BackendErrorKind::PipelineCreation,
                0x0B08,
                Recovery::Disable,
                false,
            ),
            (
                BackendErrorKind::Internal,
                0x0B09,
                Recovery::RecreateDevice,
                true,
            ),
        ];
        for (kind, code, recovery, recoverable) in golden {
            assert_eq!(kind.code(), code);
            assert_eq!(BackendErrorKind::from_code(code), Some(kind));
            assert_eq!(kind.recovery(), recovery);
            assert_eq!(kind.is_recoverable(), recoverable);
            // II.3.1 ErrorCode::Internal = 1.
            assert_eq!(kind.core_error_code(), 1);
            assert!(!kind.user_message().is_empty());
            let error = BackendError::new(kind, "detail");
            assert_eq!(error.recovery(), recovery);
            assert_eq!(error.is_recoverable(), recoverable);
        }
        assert_eq!(BackendErrorKind::from_code(0), None);
        assert_eq!(BackendErrorKind::from_code(0x0B0A), None);
    }

    fn err(kind: BackendErrorKind) -> BackendError {
        BackendError::new(kind, "test")
    }

    #[test]
    fn memory_errors_retry_exactly_once() {
        // purge 후 재시도는 한 번뿐이고, 재시도도 실패하면 loop 없이 frame을 버립니다.
        let mut tracker = RecoveryTracker::new();
        let oom = err(BackendErrorKind::OutOfDeviceMemory);
        let mut retries = 0;
        let mut action = tracker.on_error(&oom);
        while action == RecoveryAction::PurgeCachesAndRetry {
            retries += 1;
            // 재시도도 메모리 부족으로 실패.
            action = tracker.on_error(&oom);
        }
        assert_eq!(retries, 1);
        assert_eq!(action, RecoveryAction::SkipFrame);
        // 다음 작업은 다시 한 번 재시도할 수 있습니다.
        assert_eq!(tracker.on_error(&oom), RecoveryAction::PurgeCachesAndRetry);
        tracker.operation_succeeded();
        assert_eq!(
            tracker.on_error(&err(BackendErrorKind::OutOfHostMemory)),
            RecoveryAction::PurgeCachesAndRetry
        );
    }

    #[test]
    fn repeated_errors_escalate() {
        let mut tracker = RecoveryTracker::new();
        let timeout = err(BackendErrorKind::Timeout);
        assert_eq!(tracker.on_error(&timeout), RecoveryAction::RetryNextFrame);
        assert_eq!(tracker.on_error(&timeout), RecoveryAction::RetryNextFrame);
        assert_eq!(
            tracker.on_error(&timeout),
            RecoveryAction::ReconfigureSurface
        );
        // 성공한 frame이 계수를 초기화합니다.
        tracker.frame_succeeded();
        assert_eq!(tracker.on_error(&timeout), RecoveryAction::RetryNextFrame);
        tracker.frame_succeeded();

        let surface = err(BackendErrorKind::SurfaceLost);
        assert_eq!(
            tracker.on_error(&surface),
            RecoveryAction::ReconfigureSurface
        );
        assert_eq!(tracker.on_error(&surface), RecoveryAction::RecreateDevice);
        tracker.frame_succeeded();

        // device 재생성은 연속 3번까지, 그 뒤에는 비활성화(무한 재생성 loop 금지).
        let lost = err(BackendErrorKind::DeviceLost);
        for _ in 0..MAX_DEVICE_RECREATIONS {
            assert_eq!(tracker.on_error(&lost), RecoveryAction::RecreateDevice);
        }
        assert_eq!(tracker.on_error(&lost), RecoveryAction::Disable);
        assert!(tracker.is_disabled());
        // 비활성화는 성공 frame으로 풀리지 않고 이후 모든 오류도 Disable입니다.
        tracker.frame_succeeded();
        assert!(tracker.is_disabled());
        assert_eq!(tracker.on_error(&timeout), RecoveryAction::Disable);

        let mut fresh = RecoveryTracker::new();
        assert_eq!(
            fresh.on_error(&err(BackendErrorKind::InvalidUsage)),
            RecoveryAction::SkipFrame
        );
        assert_eq!(
            fresh.on_error(&err(BackendErrorKind::InvalidUsage)),
            RecoveryAction::SkipFrame
        );
        assert!(!fresh.is_disabled());
        assert_eq!(
            fresh.on_error(&err(BackendErrorKind::PipelineCreation)),
            RecoveryAction::Disable
        );
    }
}

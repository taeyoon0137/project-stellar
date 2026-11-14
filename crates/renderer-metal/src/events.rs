//! device 오류·memory pressure·drawable timeout event와 GPU 메모리 예산 (XI.8.7).
//!
//! 어떤 GPU 오류도 process를 끝내지 않습니다. backend는 오류를 [`BackendError`]로 돌려주는 동시에
//! [`MetalEvent`]를 쌓고, 앱은 [`MetalBackend::drain_events`](crate::MetalBackend::drain_events)로 꺼내
//! 진단에 남깁니다. event에는 정적 이름과 정수만 있습니다(아키텍처 §39: 렌더러 오류 코드만 진단에 남김).
//!
//! ## command buffer 오류 분류
//!
//! [`classify_command_buffer_error`]가 `MTLCommandBufferError` 코드를 XI.5.4 [`BackendErrorKind`]로
//! 바꿉니다. 복구 방법은 그 종류의 기본 정책(`recovery`)을 따릅니다.
//!
//! | `MTLCommandBufferError` | 종류 | 근거 |
//! |---|---|---|
//! | `Timeout`(2) | `Timeout` | GPU watchdog. 다음 frame 재시도 |
//! | `NotPermitted`(7) | `Timeout` | 일시적으로 GPU 사용 불가. 다음 frame 재시도 |
//! | `OutOfMemory`(8) | `OutOfDeviceMemory` | cache purge 후 재시도 |
//! | `DeviceRemoved`(11) | `DeviceLost` | device 재생성 |
//! | `AccessRevoked`(4) | `Unsupported` | process의 GPU 접근이 막힘. 재생성으로 회복 불가 → 비활성화 |
//! | `PageFault`(3)·`InvalidResource`(9)·`Memoryless`(10)·`StackOverflow`(12) | `InvalidUsage` | 잘못된 command(공통 계층 버그). frame skip + 보고 |
//! | `Internal`(1)·그 밖 | `Internal` | device 재생성 |
//!
//! ## GPU 메모리 예산
//!
//! [`MemoryBudget`]은 backend가 만든 buffer·texture·transient의 `allocatedSize` 합을 예산(기본:
//! device `recommendedMaxWorkingSetSize`)과 비교합니다. 넘는 할당은 Metal에 요청하기 전에 거부하고
//! `MemoryPressure` event와 `OutOfDeviceMemory`(복구: cache purge 후 한 번 재시도)를 돌려줍니다. 거부 전에
//! backend가 다시 만들 수 있는 transient pool부터 비웁니다.

use std::fmt::Write as _;

use stellar_renderer_core::backend::{BackendError, BackendErrorKind, FrameId, SubmissionId};

use crate::pipeline::ShaderErrorStage;

/// 보관하는 event 최대 수(넘으면 오래된 것부터 버림).
pub const MAX_EVENTS: usize = 256;

/// backend 오류 event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetalEvent {
    /// GPU device를 만들 수 없거나 command buffer 실행이 실패했습니다.
    DeviceError {
        /// 분류한 종류.
        kind: BackendErrorKind,
        /// 실패한 submission(device 생성 실패·texture upload면 `None`).
        submission: Option<SubmissionId>,
        /// `MTLCommandBufferError` 코드(없으면 `None`).
        command_buffer_error: Option<i64>,
    },
    /// GPU 메모리 예산을 넘었거나 Metal이 할당을 거부했습니다.
    MemoryPressure {
        /// 요청한 byte.
        requested_bytes: u64,
        /// 요청 시점 backend 할당 byte.
        resident_bytes: u64,
        /// 예산 byte.
        budget_bytes: u64,
    },
    /// drawable을 얻지 못해 frame을 건너뛰었습니다(가려진 창·timeout).
    DrawableTimeout {
        /// 건너뛴 frame.
        frame: FrameId,
        /// `nextDrawable` 대기 시간(ns).
        waited_nanos: u64,
    },
    /// shader compile·pipeline 생성 실패(log는 `last_shader_error`).
    PipelineFailure {
        /// 실패 단계.
        stage: ShaderErrorStage,
    },
}

impl MetalEvent {
    /// 정적 event 이름.
    pub const fn name(&self) -> &'static str {
        match self {
            MetalEvent::DeviceError { .. } => "device_error",
            MetalEvent::MemoryPressure { .. } => "memory_pressure",
            MetalEvent::DrawableTimeout { .. } => "drawable_timeout",
            MetalEvent::PipelineFailure { .. } => "pipeline_failure",
        }
    }

    /// event에 대응하는 오류 종류.
    pub const fn kind(&self) -> BackendErrorKind {
        match *self {
            MetalEvent::DeviceError { kind, .. } => kind,
            MetalEvent::MemoryPressure { .. } => BackendErrorKind::OutOfDeviceMemory,
            MetalEvent::DrawableTimeout { .. } => BackendErrorKind::Timeout,
            MetalEvent::PipelineFailure { .. } => BackendErrorKind::PipelineCreation,
        }
    }

    /// 고정 형식 한 줄(`renderer.metal.event name=.. code=.. ...`). code는 XI.5.4 안정 렌더러 오류 코드.
    pub fn to_line(&self) -> String {
        let mut line = format!(
            "renderer.metal.event name={} code={}",
            self.name(),
            self.kind().code()
        );
        // String에 쓰기는 실패하지 않습니다.
        let _ = match *self {
            MetalEvent::DeviceError {
                submission,
                command_buffer_error,
                ..
            } => write!(
                line,
                " submission={} command_buffer_error={}",
                submission.map_or(0, |id| id.0),
                command_buffer_error.unwrap_or(-1)
            ),
            MetalEvent::MemoryPressure {
                requested_bytes,
                resident_bytes,
                budget_bytes,
            } => write!(
                line,
                " requested={requested_bytes} resident={resident_bytes} budget={budget_bytes}"
            ),
            MetalEvent::DrawableTimeout {
                frame,
                waited_nanos,
            } => write!(line, " frame={} waited_ns={waited_nanos}", frame.0),
            MetalEvent::PipelineFailure { stage } => write!(line, " stage={stage:?}"),
        };
        line
    }
}

/// `MTLCommandBufferError` 코드 → 오류 종류(모듈 문서 표).
pub fn classify_command_buffer_error(code: i64) -> BackendErrorKind {
    match code {
        2 | 7 => BackendErrorKind::Timeout,
        8 => BackendErrorKind::OutOfDeviceMemory,
        11 => BackendErrorKind::DeviceLost,
        4 => BackendErrorKind::Unsupported,
        3 | 9 | 10 | 12 => BackendErrorKind::InvalidUsage,
        _ => BackendErrorKind::Internal,
    }
}

/// command buffer 실패 오류 값(정적 detail).
pub(crate) fn command_buffer_failure(kind: BackendErrorKind) -> BackendError {
    let detail = match kind {
        BackendErrorKind::Timeout => "GPU 실행이 시간 제한을 넘었습니다",
        BackendErrorKind::OutOfDeviceMemory => "GPU 실행 중 메모리가 부족합니다",
        BackendErrorKind::DeviceLost => "GPU device를 잃었습니다",
        BackendErrorKind::Unsupported => "이 process의 GPU 접근이 막혔습니다",
        BackendErrorKind::InvalidUsage => "잘못된 GPU command가 실행되었습니다",
        _ => "GPU command buffer 실행 실패",
    };
    BackendError::new(kind, detail)
}

/// backend GPU 메모리 예산.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryBudget {
    budget: u64,
    resident: u64,
}

impl MemoryBudget {
    /// 빈 예산.
    pub(crate) const fn new(budget: u64) -> MemoryBudget {
        MemoryBudget {
            budget,
            resident: 0,
        }
    }

    /// 예산.
    pub(crate) const fn budget(&self) -> u64 {
        self.budget
    }

    /// 지금 할당 byte.
    pub(crate) const fn resident(&self) -> u64 {
        self.resident
    }

    /// `release`를 놓고 `bytes`를 더해도 예산 안이면 참입니다.
    pub(crate) fn fits(&self, bytes: u64, release: u64) -> bool {
        self.resident
            .saturating_sub(release)
            .checked_add(bytes)
            .is_some_and(|total| total <= self.budget)
    }

    /// 할당을 기록합니다.
    pub(crate) fn add(&mut self, bytes: u64) {
        self.resident = self.resident.saturating_add(bytes);
    }

    /// 해제를 기록합니다.
    pub(crate) fn remove(&mut self, bytes: u64) {
        self.resident = self.resident.saturating_sub(bytes);
    }

    /// 요청에 대한 pressure event.
    pub(crate) const fn pressure(&self, requested: u64) -> MetalEvent {
        MetalEvent::MemoryPressure {
            requested_bytes: requested,
            resident_bytes: self.resident,
            budget_bytes: self.budget,
        }
    }
}

#[cfg(test)]
mod tests {
    use stellar_renderer_core::backend::recovery::Recovery;

    use super::*;

    #[test]
    fn command_buffer_error_matrix() {
        // 검증 기준(XI.8.7): GPU 오류 코드마다 분류와 복구 정책이 고정됩니다.
        let cases = [
            (0, BackendErrorKind::Internal, Recovery::RecreateDevice),
            (1, BackendErrorKind::Internal, Recovery::RecreateDevice),
            (2, BackendErrorKind::Timeout, Recovery::RetryNextFrame),
            (
                3,
                BackendErrorKind::InvalidUsage,
                Recovery::SkipFrameAndReport,
            ),
            (4, BackendErrorKind::Unsupported, Recovery::Disable),
            (7, BackendErrorKind::Timeout, Recovery::RetryNextFrame),
            (
                8,
                BackendErrorKind::OutOfDeviceMemory,
                Recovery::PurgeCachesAndRetryOnce,
            ),
            (
                9,
                BackendErrorKind::InvalidUsage,
                Recovery::SkipFrameAndReport,
            ),
            (
                10,
                BackendErrorKind::InvalidUsage,
                Recovery::SkipFrameAndReport,
            ),
            (11, BackendErrorKind::DeviceLost, Recovery::RecreateDevice),
            (
                12,
                BackendErrorKind::InvalidUsage,
                Recovery::SkipFrameAndReport,
            ),
            (999, BackendErrorKind::Internal, Recovery::RecreateDevice),
            (-5, BackendErrorKind::Internal, Recovery::RecreateDevice),
        ];
        for (code, kind, recovery) in cases {
            assert_eq!(classify_command_buffer_error(code), kind, "code {code}");
            let error = command_buffer_failure(kind);
            assert_eq!(error.kind(), kind);
            assert_eq!(error.recovery(), recovery, "code {code}");
        }
    }

    #[test]
    fn event_lines_use_stable_codes() {
        let device = MetalEvent::DeviceError {
            kind: BackendErrorKind::DeviceLost,
            submission: Some(SubmissionId(4)),
            command_buffer_error: Some(11),
        };
        assert_eq!(
            device.to_line(),
            "renderer.metal.event name=device_error code=2819 submission=4 command_buffer_error=11"
        );
        let budget = MemoryBudget {
            budget: 100,
            resident: 60,
        };
        assert_eq!(
            budget.pressure(50).to_line(),
            "renderer.metal.event name=memory_pressure code=2820 requested=50 resident=60 budget=100"
        );
        assert_eq!(
            MetalEvent::DrawableTimeout {
                frame: FrameId(9),
                waited_nanos: 1_000
            }
            .to_line(),
            "renderer.metal.event name=drawable_timeout code=2823 frame=9 waited_ns=1000"
        );
        assert_eq!(
            MetalEvent::PipelineFailure {
                stage: ShaderErrorStage::Compile
            }
            .to_line(),
            "renderer.metal.event name=pipeline_failure code=2824 stage=Compile"
        );
        assert!(budget.fits(40, 0) && !budget.fits(41, 0) && budget.fits(60, 20));
        assert!(!budget.fits(u64::MAX, 0));
    }
}

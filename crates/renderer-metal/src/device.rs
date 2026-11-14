//! `MTLDevice`·`MTLCommandQueue` 초기화와 capability 조회 (XI.8.1).

use stellar_renderer_core::backend::{
    BackendError, BackendErrorKind, BackendKind, DeviceCapabilities,
};
use stellar_renderer_core::render_graph::MAX_TEXTURE_DIMENSION;

use crate::objc::{sel, Id, NSUInteger};
use crate::sys::MTLCreateSystemDefaultDevice;

/// GPU device와 command queue.
#[derive(Debug)]
pub(crate) struct MetalDevice {
    device: Id,
    queue: Id,
}

impl MetalDevice {
    /// 시스템 기본 GPU와 command queue를 만듭니다.
    ///
    /// GPU가 없으면 `Unsupported`, queue를 만들지 못하면 `OutOfDeviceMemory`입니다.
    pub(crate) fn system_default() -> Result<MetalDevice, BackendError> {
        // SAFETY: Create 규칙 함수로 +1 device 또는 nil을 돌려줍니다.
        let device = unsafe { Id::from_owned(MTLCreateSystemDefaultDevice()) }.ok_or(
            BackendError::new(BackendErrorKind::Unsupported, "Metal GPU device가 없습니다"),
        )?;
        // SAFETY: `-newCommandQueue`는 () -> id<MTLCommandQueue>(+1, 실패 시 nil)입니다.
        let queue = unsafe { Id::from_owned(device.send(sel!("newCommandQueue"), ())) }.ok_or(
            BackendError::new(
                BackendErrorKind::OutOfDeviceMemory,
                "Metal command queue를 만들 수 없습니다",
            ),
        )?;
        Ok(MetalDevice { device, queue })
    }

    /// `id<MTLDevice>`.
    pub(crate) fn device(&self) -> &Id {
        &self.device
    }

    /// `id<MTLCommandQueue>`.
    pub(crate) fn queue(&self) -> &Id {
        &self.queue
    }

    /// 권장 최대 작업 집합 크기(`recommendedMaxWorkingSetSize`, byte). GPU 메모리 예산 기본값입니다.
    pub(crate) fn recommended_working_set(&self) -> u64 {
        // SAFETY: `-recommendedMaxWorkingSetSize`는 () -> uint64_t입니다.
        unsafe { self.device.send(sel!("recommendedMaxWorkingSetSize"), ()) }
    }

    /// `-supportsTextureSampleCount:`.
    pub(crate) fn supports_sample_count(&self, count: NSUInteger) -> bool {
        // SAFETY: `-supportsTextureSampleCount:`은 (NSUInteger) -> BOOL입니다.
        unsafe {
            self.device
                .send(sel!("supportsTextureSampleCount:"), (count,))
        }
    }

    /// device capability.
    ///
    /// - Apple Silicon(ADR-0006)의 2D texture 한 변 최대값은 16384이고 공통 상한
    ///   ([`MAX_TEXTURE_DIMENSION`])과 같습니다.
    /// - `display_p3_surface`: CAMetalLayer는 colorspace로 Display P3를 지정할 수 있으므로 layer가 있으면
    ///   참입니다. 실제 display gamut 감지는 XI.10.5의 platform capability입니다. headless에는 surface가
    ///   없으므로 거짓입니다.
    /// - `stencil_bits`: `Depth32Float_Stencil8`의 stencil 8 bit.
    pub(crate) fn capabilities(&self, has_surface: bool, timestamps: bool) -> DeviceCapabilities {
        DeviceCapabilities {
            backend: BackendKind::Metal,
            max_texture_dimension: MAX_TEXTURE_DIMENSION,
            display_p3_surface: has_surface,
            timestamp_query: timestamps,
            msaa_x4: self.supports_sample_count(4),
            stencil_bits: 8,
        }
    }
}

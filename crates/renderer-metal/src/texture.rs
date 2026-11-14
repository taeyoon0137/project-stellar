//! GPU texture 생성과 descriptor 검증 (XI.8.1).
//!
//! 모든 texture는 `MTLStorageModePrivate`(GPU 전용)입니다. CPU 읽기·쓰기는 blit으로 합니다.
//! descriptor는 Metal에 넘기기 전에 검사합니다. 잘못된 descriptor는 Metal 내부 assertion으로 process를
//! 끝낼 수 있으므로 공통 계층 실수도 `InvalidUsage`로 돌려줍니다.

use stellar_renderer_core::backend::{BackendError, BackendErrorKind, GpuTextureDesc};
use stellar_renderer_core::render_graph::{SampleCount, MAX_TEXTURE_DIMENSION};

use crate::format::mtl_pixel_format;
use crate::objc::{class, sel, send, Id, NSUInteger};
use crate::sys::{storage_mode, texture_type, texture_usage};

/// backend가 보관하는 texture.
#[derive(Debug)]
pub(crate) struct MetalTexture {
    /// `id<MTLTexture>`.
    pub(crate) texture: Id,
    /// 생성 descriptor.
    pub(crate) desc: GpuTextureDesc,
}

/// `width × height` texture의 최대 mip level 수.
pub(crate) fn max_mip_levels(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// sample 수.
pub(crate) const fn sample_count(samples: SampleCount) -> NSUInteger {
    match samples {
        SampleCount::X1 => 1,
        SampleCount::X4 => 4,
    }
}

/// descriptor를 검사합니다.
pub(crate) fn validate_texture_desc(desc: &GpuTextureDesc) -> Result<(), BackendError> {
    let invalid = |detail| Err(BackendError::new(BackendErrorKind::InvalidUsage, detail));
    if desc.width == 0 || desc.height == 0 {
        return invalid("texture 크기는 0일 수 없습니다");
    }
    if desc.width > MAX_TEXTURE_DIMENSION || desc.height > MAX_TEXTURE_DIMENSION {
        return invalid("texture 크기가 상한을 넘습니다");
    }
    if desc.mip_levels == 0 || desc.mip_levels > max_mip_levels(desc.width, desc.height) {
        return invalid("texture mip level 수가 범위를 벗어났습니다");
    }
    if !desc.usage.sampled && !desc.usage.render_target {
        return invalid("texture 용도가 없습니다");
    }
    if desc.samples == SampleCount::X4 && (desc.mip_levels != 1 || desc.usage.sampled) {
        return invalid("multisample texture는 mip 1개, render target 전용입니다");
    }
    if desc.format.is_depth_stencil() && desc.mip_levels != 1 {
        return invalid("depth·stencil texture는 mip 1개입니다");
    }
    Ok(())
}

/// 검사한 descriptor로 private texture를 만듭니다. Metal이 nil이면 `OutOfDeviceMemory`.
pub(crate) fn create_texture(device: &Id, desc: &GpuTextureDesc) -> Result<Id, BackendError> {
    validate_texture_desc(desc)?;
    let class = class!("MTLTextureDescriptor").ok_or(BackendError::new(
        BackendErrorKind::Unsupported,
        "MTLTextureDescriptor class가 없습니다",
    ))?;
    // SAFETY: `+new`는 () -> id(+1)입니다.
    let descriptor = unsafe { Id::from_owned(send(class.as_raw(), sel!("new"), ())) }.ok_or(
        BackendError::new(
            BackendErrorKind::OutOfHostMemory,
            "texture descriptor를 만들 수 없습니다",
        ),
    )?;
    let kind = match desc.samples {
        SampleCount::X1 => texture_type::TYPE_2D,
        SampleCount::X4 => texture_type::TYPE_2D_MULTISAMPLE,
    };
    let mut usage = 0;
    if desc.usage.sampled {
        usage |= texture_usage::SHADER_READ;
    }
    if desc.usage.render_target {
        usage |= texture_usage::RENDER_TARGET;
    }
    // SAFETY: MTLTextureDescriptor setter는 모두 (NSUInteger) -> void이고 값은 위에서 검사했습니다.
    // `-newTextureWithDescriptor:`는 (MTLTextureDescriptor*) -> id<MTLTexture>(+1, nil 가능)입니다.
    unsafe {
        let _: () = descriptor.send(sel!("setTextureType:"), (kind,));
        let _: () = descriptor.send(sel!("setPixelFormat:"), (mtl_pixel_format(desc.format),));
        let _: () = descriptor.send(sel!("setWidth:"), (desc.width as NSUInteger,));
        let _: () = descriptor.send(sel!("setHeight:"), (desc.height as NSUInteger,));
        let _: () = descriptor.send(
            sel!("setMipmapLevelCount:"),
            (desc.mip_levels as NSUInteger,),
        );
        let _: () = descriptor.send(sel!("setSampleCount:"), (sample_count(desc.samples),));
        let _: () = descriptor.send(sel!("setUsage:"), (usage,));
        let _: () = descriptor.send(sel!("setStorageMode:"), (storage_mode::PRIVATE,));
        Id::from_owned(device.send(sel!("newTextureWithDescriptor:"), (descriptor.as_raw(),)))
    }
    .ok_or(BackendError::new(
        BackendErrorKind::OutOfDeviceMemory,
        "GPU texture를 할당할 수 없습니다",
    ))
}

#[cfg(test)]
mod tests {
    use stellar_renderer_core::backend::TextureUsage;
    use stellar_renderer_core::render_graph::TextureFormat;

    use super::*;

    fn desc(width: u32, height: u32, mips: u32) -> GpuTextureDesc {
        GpuTextureDesc {
            width,
            height,
            format: TextureFormat::Rgba8Unorm,
            mip_levels: mips,
            samples: SampleCount::X1,
            usage: TextureUsage {
                sampled: true,
                render_target: false,
            },
        }
    }

    #[test]
    fn descriptor_rules() {
        assert_eq!(max_mip_levels(1, 1), 1);
        assert_eq!(max_mip_levels(16, 9), 5);
        assert_eq!(max_mip_levels(16_384, 1), 15);
        assert!(validate_texture_desc(&desc(16, 9, 5)).is_ok());
        let rejected = [
            desc(0, 4, 1),
            desc(16_385, 4, 1),
            desc(16, 9, 6),
            desc(4, 4, 0),
            GpuTextureDesc {
                usage: TextureUsage {
                    sampled: false,
                    render_target: false,
                },
                ..desc(4, 4, 1)
            },
            GpuTextureDesc {
                samples: SampleCount::X4,
                ..desc(4, 4, 1)
            },
            GpuTextureDesc {
                format: TextureFormat::Depth32FloatStencil8,
                ..desc(4, 4, 2)
            },
        ];
        for bad in rejected {
            let error = validate_texture_desc(&bad).unwrap_err();
            assert_eq!(error.kind(), BackendErrorKind::InvalidUsage, "{bad:?}");
        }
        let msaa = GpuTextureDesc {
            samples: SampleCount::X4,
            usage: TextureUsage {
                sampled: false,
                render_target: true,
            },
            ..desc(8, 8, 1)
        };
        assert!(validate_texture_desc(&msaa).is_ok());
    }
}

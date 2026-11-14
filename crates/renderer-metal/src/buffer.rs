//! 공통 vertex/index/uniform/staging buffer → `MTLBuffer` 대응과 texture upload (XI.8.3).
//!
//! ## buffer
//!
//! 모든 [`BufferUsage`]는 `MTLStorageModeShared` buffer 하나로 대응합니다(Apple Silicon 통합 메모리,
//! ADR-0006). [`update_buffer`](stellar_renderer_core::backend::RendererBackend::update_buffer)는
//! CPU 즉시 쓰기입니다. GPU가 아직 그 범위를 읽는 submission이 있으면 덮어쓰지 않는 것은 공통 계층
//! frame-local ring(XI.5.3)의 책임입니다.
//!
//! ## texture upload
//!
//! texture는 private storage라 CPU가 직접 쓰지 못합니다. `update_texture`는 data를 staging buffer로
//! 복사해 [`PendingUpload`]로 쌓고, 다음 `encode`(또는 readback) 시작 때 blit command buffer 하나로
//! 한꺼번에 제출합니다. 같은 queue에서 frame command buffer보다 먼저 commit하므로 frame은 upload 결과를
//! 봅니다(queue 순서). 따라서 texture upload는 CPU timeline이 아니라 GPU queue 순서를 따르고, 이미
//! 제출한 frame이 읽는 내용을 바꾸지 않습니다.

use std::ffi::c_void;
use std::ptr::NonNull;

use stellar_renderer_core::backend::{
    BackendError, BackendErrorKind, BufferDesc, BufferUsage, TextureRegion,
};
use stellar_renderer_core::render_graph::SampleCount;

use crate::format::bytes_per_pixel;
use crate::objc::{sel, send, Id, NSUInteger, RawId};
use crate::sys::{resource_options, MTLOrigin, MTLSize};
use crate::texture::{allocated_size, MetalTexture};

fn invalid(detail: &'static str) -> BackendError {
    BackendError::new(BackendErrorKind::InvalidUsage, detail)
}

/// backend가 보관하는 buffer.
#[derive(Debug)]
pub(crate) struct MetalBuffer {
    /// `id<MTLBuffer>`.
    pub(crate) buffer: Id,
    /// 생성 descriptor.
    pub(crate) desc: BufferDesc,
    /// GPU 메모리 크기(`allocatedSize`, 예산 계산용).
    pub(crate) bytes: u64,
    contents: NonNull<u8>,
}

impl MetalBuffer {
    /// shared buffer를 만듭니다. device 상한을 넘거나 Metal이 nil이면 `OutOfDeviceMemory`.
    pub(crate) fn create(device: &Id, desc: &BufferDesc) -> Result<MetalBuffer, BackendError> {
        if desc.size == 0 {
            return Err(invalid("0 크기 buffer"));
        }
        // SAFETY: `-maxBufferLength`는 () -> NSUInteger입니다.
        let max: NSUInteger = unsafe { device.send(sel!("maxBufferLength"), ()) };
        let length = usize::try_from(desc.size)
            .ok()
            .filter(|length| *length <= max)
            .ok_or(BackendError::new(
                BackendErrorKind::OutOfDeviceMemory,
                "buffer 크기가 device 상한을 넘습니다",
            ))?;
        // SAFETY: `-newBufferWithLength:options:`는 (NSUInteger, NSUInteger) -> id(+1, nil 가능)이고,
        // shared buffer의 `-contents`는 () -> void*(buffer 수명 동안 유효)입니다.
        unsafe {
            let buffer = Id::from_owned(device.send(
                sel!("newBufferWithLength:options:"),
                (length, resource_options::STORAGE_SHARED),
            ))
            .ok_or(BackendError::new(
                BackendErrorKind::OutOfDeviceMemory,
                "GPU buffer를 할당할 수 없습니다",
            ))?;
            let contents: *mut c_void = buffer.send(sel!("contents"), ());
            let contents = NonNull::new(contents.cast::<u8>()).ok_or(BackendError::new(
                BackendErrorKind::Internal,
                "buffer 내용에 접근할 수 없습니다",
            ))?;
            let bytes = allocated_size(&buffer);
            Ok(MetalBuffer {
                buffer,
                desc: *desc,
                bytes,
                contents,
            })
        }
    }

    /// `offset`부터 `data`를 씁니다(범위 검사).
    pub(crate) fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), BackendError> {
        let end = offset
            .checked_add(data.len() as u64)
            .filter(|end| *end <= self.desc.size)
            .ok_or(invalid("buffer 범위 밖 쓰기"))?;
        debug_assert!(end <= self.desc.size);
        // SAFETY: [offset, end)는 buffer 길이 안이고(위 검사, 길이 = desc.size), contents는 buffer가
        // 살아 있는 동안 유효한 쓰기 가능 메모리입니다. data는 별도 Rust slice라 겹치지 않습니다.
        unsafe {
            std::ptr::copy_nonoverlapping(
                data.as_ptr(),
                self.contents.as_ptr().add(offset as usize),
                data.len(),
            );
        }
        Ok(())
    }

    /// `byte_offset`부터 u32 index `count`개의 (최솟값, 최댓값). 범위가 buffer 밖이면 `None`,
    /// 0개면 `(0, 0)`.
    pub(crate) fn index_bounds(&self, byte_offset: u64, count: u32) -> Option<(u32, u32)> {
        let end = byte_offset.checked_add(u64::from(count).checked_mul(4)?)?;
        if end > self.desc.size {
            return None;
        }
        let mut bounds: Option<(u32, u32)> = None;
        for index in 0..u64::from(count) {
            // SAFETY: byte_offset + index*4 + 4 ≤ end ≤ buffer 길이(위 검사). GPU가 같은 buffer를 읽어도
            // CPU 읽기는 경합하지 않습니다(쓰기는 공통 계층이 GPU 완료 뒤에만 함).
            let value = unsafe {
                std::ptr::read_unaligned(
                    self.contents
                        .as_ptr()
                        .add((byte_offset + index * 4) as usize)
                        .cast::<u32>(),
                )
            };
            bounds = Some(bounds.map_or((value, value), |(low, high)| {
                (low.min(value), high.max(value))
            }));
        }
        Some(bounds.unwrap_or((0, 0)))
    }

    /// 용도가 `usage`인지 확인합니다.
    pub(crate) fn expect_usage(&self, usage: BufferUsage) -> Result<(), BackendError> {
        if self.desc.usage == usage {
            Ok(())
        } else {
            Err(invalid("buffer 용도가 command와 다릅니다"))
        }
    }
}

/// 제출을 기다리는 texture upload.
#[derive(Debug)]
pub(crate) struct PendingUpload {
    staging: Id,
    texture: Id,
    region: TextureRegion,
    bytes_per_row: usize,
    length: usize,
}

/// upload 영역을 검사하고 data를 staging buffer에 복사합니다.
pub(crate) fn stage_texture_upload(
    device: &Id,
    texture: &MetalTexture,
    region: TextureRegion,
    data: &[u8],
) -> Result<PendingUpload, BackendError> {
    let desc = texture.desc;
    let bytes_per_pixel = bytes_per_pixel(desc.format)
        .filter(|_| desc.samples == SampleCount::X1)
        .ok_or(invalid(
            "multisample·depth texture에는 upload할 수 없습니다",
        ))?;
    if region.mip_level >= desc.mip_levels {
        return Err(invalid("없는 mip level"));
    }
    let mip_width = (desc.width >> region.mip_level).max(1);
    let mip_height = (desc.height >> region.mip_level).max(1);
    let fits = |start: u32, extent: u32, limit: u32| {
        extent > 0 && start.checked_add(extent).is_some_and(|end| end <= limit)
    };
    if !fits(region.x, region.width, mip_width) || !fits(region.y, region.height, mip_height) {
        return Err(invalid("upload 영역이 texture 밖입니다"));
    }
    let bytes_per_row = region.width as usize * bytes_per_pixel as usize;
    let length = bytes_per_row * region.height as usize;
    if data.len() != length {
        return Err(invalid("upload data 길이가 영역과 다릅니다"));
    }
    // SAFETY: `-newBufferWithBytes:length:options:`는 (const void*, NSUInteger, NSUInteger) ->
    // id(+1, nil 가능)이고 data를 length byte 복사합니다.
    let staging = unsafe {
        Id::from_owned(device.send(
            sel!("newBufferWithBytes:length:options:"),
            (
                data.as_ptr().cast::<c_void>(),
                length as NSUInteger,
                resource_options::STORAGE_SHARED,
            ),
        ))
    }
    .ok_or(BackendError::new(
        BackendErrorKind::OutOfDeviceMemory,
        "upload staging buffer를 할당할 수 없습니다",
    ))?;
    Ok(PendingUpload {
        staging,
        texture: texture.texture.clone(),
        region,
        bytes_per_row,
        length,
    })
}

/// 쌓인 upload를 blit command buffer 하나에 기록해 commit하고 그 command buffer를 돌려줍니다.
///
/// upload가 없으면 `Ok(None)`.
pub(crate) fn submit_uploads(
    queue: &Id,
    uploads: &mut Vec<PendingUpload>,
) -> Result<Option<Id>, BackendError> {
    if uploads.is_empty() {
        return Ok(None);
    }
    // SAFETY: `-commandBuffer`는 () -> id(+0), `-blitCommandEncoder`는 () -> id(+0)입니다.
    // copyFromBuffer:...toTexture: 인자는 (buffer, offset, bytesPerRow, bytesPerImage, MTLSize,
    // texture, slice, level, MTLOrigin)이고 영역은 stage_texture_upload가 검사했습니다.
    unsafe {
        let command_buffer =
            Id::from_borrowed(queue.send(sel!("commandBuffer"), ())).ok_or(BackendError::new(
                BackendErrorKind::Internal,
                "upload command buffer를 만들 수 없습니다",
            ))?;
        let blit: RawId = command_buffer.send(sel!("blitCommandEncoder"), ());
        if blit.is_null() {
            return Err(BackendError::new(
                BackendErrorKind::Internal,
                "blit encoder를 만들 수 없습니다",
            ));
        }
        for upload in uploads.iter() {
            let _: () = send(
                blit,
                sel!("copyFromBuffer:sourceOffset:sourceBytesPerRow:sourceBytesPerImage:sourceSize:toTexture:destinationSlice:destinationLevel:destinationOrigin:"),
                (
                    upload.staging.as_raw(),
                    0 as NSUInteger,
                    upload.bytes_per_row as NSUInteger,
                    upload.length as NSUInteger,
                    MTLSize {
                        width: upload.region.width as NSUInteger,
                        height: upload.region.height as NSUInteger,
                        depth: 1,
                    },
                    upload.texture.as_raw(),
                    0 as NSUInteger,
                    upload.region.mip_level as NSUInteger,
                    MTLOrigin {
                        x: upload.region.x as NSUInteger,
                        y: upload.region.y as NSUInteger,
                        z: 0,
                    },
                ),
            );
        }
        let _: () = send(blit, sel!("endEncoding"), ());
        let _: () = command_buffer.send(sel!("commit"), ());
        // command buffer가 staging·texture를 retain하므로 우리 참조는 지금 놓아도 됩니다.
        uploads.clear();
        Ok(Some(command_buffer))
    }
}

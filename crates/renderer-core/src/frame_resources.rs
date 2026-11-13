//! resource upload queue와 frame-local allocator (XI.5.3).
//!
//! CPU가 frame N+1을 기록하는 동안 GPU는 frame N, N−1, …을 실행합니다(frames in flight, 최대 3).
//! 이 모듈은 GPU가 아직 읽는 메모리·resource를 CPU가 덮어쓰거나 해제하지 않게(premature free 방지)
//! 하는 세 가지 도구를 둡니다.
//!
//! - [`FrameLocalAllocator`]·[`RingFrameAllocator`]: frame마다 쓰고 버리는 uniform·vertex 데이터용
//!   bump allocator입니다. frames in flight 수만큼 slot을 돌려 쓰고, slot은 그 slot을 쓴 submission이
//!   GPU에서 끝난 뒤([`FrameLocalAllocator::retire`])에만 다시 엽니다. 아직 GPU가 쓰는 slot을 열려 하면
//!   [`FrameAllocError::SlotBusy`]이며, 호출자는 GPU 진행을 기다린 뒤 다시 시도합니다.
//! - [`DeferredRelease`]: 파괴 요청한 GPU resource를, 그것을 참조했을 수 있는 마지막 submission이
//!   끝날 때까지 보관했다가 돌려줍니다. 기록 중인 frame이 참조했을 수 있으면 그 frame의 submission까지
//!   기다립니다. 공통 계층은 돌려받은 resource에만 `destroy_*`를 호출합니다.
//! - [`UploadQueue`]: texture·buffer upload를 FIFO로 모아 frame encode **전**에 byte 예산만큼
//!   backend에 넘깁니다. 큰 upload가 interaction frame을 막지 않게 여러 frame에 나눕니다. 예산보다 큰
//!   요청도 한 번에 하나는 내보내 진행을 보장합니다.
//!
//! GPU는 단일 queue에서 submission을 제출 순서대로 끝낸다고 가정합니다. 따라서 "submission `s`가
//! 끝났다"는 `s` 이하가 모두 끝났다는 뜻입니다([`GpuProgress::completed`]).
//!
//! [`GpuProgress::completed`]: crate::backend::GpuProgress::completed

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use crate::backend::{
    BackendError, BufferHandle, FrameId, RendererBackend, SubmissionId, TextureHandle,
    TextureRegion, MAX_FRAMES_IN_FLIGHT,
};
use crate::render_graph::TextureFormat;

/// frame-local allocator 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameAllocError {
    /// frames in flight가 1..=3이 아닙니다.
    InvalidFramesInFlight {
        /// 요청 값.
        requested: u8,
    },
    /// slot 용량이 0입니다.
    InvalidCapacity,
    /// alignment가 2의 거듭제곱(1..=256)이 아닙니다.
    InvalidAlignment {
        /// 요청 alignment.
        align: u64,
    },
    /// 0 byte 할당.
    ZeroSize,
    /// 다음 slot을 GPU가 아직 씁니다. 해당 submission 완료 후 다시 시도합니다.
    SlotBusy {
        /// slot.
        slot: u8,
        /// slot을 쓰는 submission.
        submission: SubmissionId,
    },
    /// 이미 frame을 기록 중입니다.
    FrameAlreadyOpen,
    /// 기록 중인 frame이 없습니다.
    NoOpenFrame,
    /// slot 남은 공간이 부족합니다(호출자는 전용 buffer로 대체).
    OutOfSpace {
        /// 요청 크기.
        requested: u64,
        /// 남은 공간(alignment 반영 전).
        remaining: u64,
    },
}

impl fmt::Display for FrameAllocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameAllocError::InvalidFramesInFlight { requested } => write!(
                f,
                "frames in flight는 1..={MAX_FRAMES_IN_FLIGHT}이어야 합니다: {requested}"
            ),
            FrameAllocError::InvalidCapacity => f.write_str("frame slot 용량은 0보다 커야 합니다"),
            FrameAllocError::InvalidAlignment { align } => {
                write!(
                    f,
                    "alignment는 1..=256의 2의 거듭제곱이어야 합니다: {align}"
                )
            }
            FrameAllocError::ZeroSize => f.write_str("0 byte 할당"),
            FrameAllocError::SlotBusy { slot, submission } => write!(
                f,
                "frame slot {slot}을 submission {}이 아직 씁니다",
                submission.0
            ),
            FrameAllocError::FrameAlreadyOpen => f.write_str("이미 frame을 기록 중입니다"),
            FrameAllocError::NoOpenFrame => f.write_str("기록 중인 frame이 없습니다"),
            FrameAllocError::OutOfSpace {
                requested,
                remaining,
            } => write!(
                f,
                "frame slot 공간 부족: 요청 {requested}, 남음 {remaining}"
            ),
        }
    }
}

impl std::error::Error for FrameAllocError {}

/// frame-local 할당 결과. backend는 slot마다 같은 크기의 buffer를 하나씩 둡니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameAllocation {
    /// frame slot(= slot buffer index).
    pub slot: u8,
    /// slot buffer 안 byte offset(요청 alignment로 정렬).
    pub offset: u64,
    /// byte 크기.
    pub size: u64,
}

/// frame-local allocator interface.
///
/// 한 frame의 수명: `begin_frame` → `allocate`* → (`end_frame(submission)` | `abandon_frame`).
/// `end_frame`한 slot은 `retire`가 그 submission 완료를 알릴 때까지 다시 열리지 않습니다.
pub trait FrameLocalAllocator {
    /// frame 기록을 시작하고 사용할 slot을 엽니다.
    fn begin_frame(&mut self, frame: FrameId) -> Result<u8, FrameAllocError>;

    /// 현재 frame slot에서 `size` byte를 `align` 정렬로 할당합니다.
    fn allocate(&mut self, size: u64, align: u64) -> Result<FrameAllocation, FrameAllocError>;

    /// frame을 submission에 넘깁니다. slot은 그 submission 완료까지 사용 중입니다.
    fn end_frame(&mut self, submission: SubmissionId) -> Result<(), FrameAllocError>;

    /// 제출하지 않은 frame을 버립니다(GPU가 보지 않았으므로 slot을 바로 비움).
    fn abandon_frame(&mut self) -> Result<(), FrameAllocError>;

    /// `completed` 이하 submission이 쓰던 slot을 비웁니다.
    fn retire(&mut self, completed: SubmissionId);

    /// device lost: GPU 작업이 모두 사라졌으므로 모든 slot을 비웁니다.
    fn reset(&mut self);
}

/// slot 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SlotState {
    Free,
    Recording(FrameId),
    InFlight(SubmissionId),
}

/// frames in flight 수만큼 slot을 돌려 쓰는 bump allocator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RingFrameAllocator {
    slots: Vec<SlotState>,
    capacity: u64,
    cursor: u64,
    current: Option<usize>,
    next: usize,
}

impl RingFrameAllocator {
    /// slot `frames_in_flight`개, slot당 `capacity` byte.
    pub fn new(frames_in_flight: u8, capacity: u64) -> Result<RingFrameAllocator, FrameAllocError> {
        if frames_in_flight == 0 || frames_in_flight > MAX_FRAMES_IN_FLIGHT {
            return Err(FrameAllocError::InvalidFramesInFlight {
                requested: frames_in_flight,
            });
        }
        if capacity == 0 {
            return Err(FrameAllocError::InvalidCapacity);
        }
        Ok(RingFrameAllocator {
            slots: vec![SlotState::Free; usize::from(frames_in_flight)],
            capacity,
            cursor: 0,
            current: None,
            next: 0,
        })
    }

    /// slot 수.
    pub fn slot_count(&self) -> u8 {
        self.slots.len() as u8
    }

    /// slot당 용량.
    pub const fn capacity(&self) -> u64 {
        self.capacity
    }

    /// GPU가 아직 쓰는 slot 수.
    pub fn in_flight(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| matches!(slot, SlotState::InFlight(_)))
            .count()
    }

    /// 현재 frame에서 할당한 byte(정렬 padding 포함).
    pub const fn used(&self) -> u64 {
        self.cursor
    }
}

impl FrameLocalAllocator for RingFrameAllocator {
    fn begin_frame(&mut self, frame: FrameId) -> Result<u8, FrameAllocError> {
        if self.current.is_some() {
            return Err(FrameAllocError::FrameAlreadyOpen);
        }
        let slot = self.next;
        if let SlotState::InFlight(submission) = self.slots[slot] {
            return Err(FrameAllocError::SlotBusy {
                slot: slot as u8,
                submission,
            });
        }
        self.slots[slot] = SlotState::Recording(frame);
        self.current = Some(slot);
        self.cursor = 0;
        Ok(slot as u8)
    }

    fn allocate(&mut self, size: u64, align: u64) -> Result<FrameAllocation, FrameAllocError> {
        let slot = self.current.ok_or(FrameAllocError::NoOpenFrame)?;
        if size == 0 {
            return Err(FrameAllocError::ZeroSize);
        }
        if !align.is_power_of_two() || align > 256 {
            return Err(FrameAllocError::InvalidAlignment { align });
        }
        let out_of_space = FrameAllocError::OutOfSpace {
            requested: size,
            remaining: self.capacity - self.cursor,
        };
        let offset = self
            .cursor
            .checked_add(align - 1)
            .map(|value| value & !(align - 1))
            .ok_or(out_of_space)?;
        let end = offset.checked_add(size).ok_or(out_of_space)?;
        if end > self.capacity {
            return Err(out_of_space);
        }
        self.cursor = end;
        Ok(FrameAllocation {
            slot: slot as u8,
            offset,
            size,
        })
    }

    fn end_frame(&mut self, submission: SubmissionId) -> Result<(), FrameAllocError> {
        let slot = self.current.take().ok_or(FrameAllocError::NoOpenFrame)?;
        self.slots[slot] = SlotState::InFlight(submission);
        self.next = (slot + 1) % self.slots.len();
        self.cursor = 0;
        Ok(())
    }

    fn abandon_frame(&mut self) -> Result<(), FrameAllocError> {
        let slot = self.current.take().ok_or(FrameAllocError::NoOpenFrame)?;
        // GPU가 보지 않은 slot이므로 바로 비우고 다음 frame이 같은 slot을 씁니다.
        self.slots[slot] = SlotState::Free;
        self.cursor = 0;
        Ok(())
    }

    fn retire(&mut self, completed: SubmissionId) {
        for slot in &mut self.slots {
            if let SlotState::InFlight(submission) = *slot {
                if submission <= completed {
                    *slot = SlotState::Free;
                }
            }
        }
    }

    fn reset(&mut self) {
        for slot in &mut self.slots {
            *slot = SlotState::Free;
        }
        self.current = None;
        self.cursor = 0;
        self.next = 0;
    }
}

/// GPU 사용이 끝날 때까지 resource 해제를 미루는 queue.
///
/// frame 경계 알림(`frame_begun`·`frame_submitted`·`frame_abandoned`)과 GPU 완료(`collect`)로
/// "이 resource를 참조했을 수 있는 마지막 submission"을 추적합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeferredRelease<T> {
    /// 기록 중인 frame 동안 파괴 요청된 항목(그 frame의 submission까지 대기).
    recording: Option<Vec<T>>,
    /// submission 완료를 기다리는 항목(submission 오름차순).
    waiting: VecDeque<(SubmissionId, T)>,
    /// 바로 해제해도 되는 항목.
    ready: Vec<T>,
    last_submitted: Option<SubmissionId>,
    completed: Option<SubmissionId>,
}

impl<T> Default for DeferredRelease<T> {
    fn default() -> DeferredRelease<T> {
        DeferredRelease {
            recording: None,
            waiting: VecDeque::new(),
            ready: Vec::new(),
            last_submitted: None,
            completed: None,
        }
    }
}

impl<T> DeferredRelease<T> {
    /// 빈 queue.
    pub fn new() -> DeferredRelease<T> {
        DeferredRelease::default()
    }

    /// 아직 해제하지 못한 항목 수.
    pub fn pending(&self) -> usize {
        self.recording.as_ref().map_or(0, Vec::len) + self.waiting.len() + self.ready.len()
    }

    fn after_last_submission(&mut self, item: T) {
        match self.last_submitted {
            Some(last) if self.completed.is_none_or(|done| done < last) => {
                self.waiting.push_back((last, item));
            }
            _ => self.ready.push(item),
        }
    }

    /// 파괴를 요청합니다. 기록 중인 frame이 있으면 그 frame의 submission까지, 없으면 마지막
    /// submission 완료까지 미룹니다.
    pub fn defer(&mut self, item: T) {
        match self.recording.as_mut() {
            Some(items) => items.push(item),
            None => self.after_last_submission(item),
        }
    }

    /// frame 기록 시작.
    pub fn frame_begun(&mut self) {
        if self.recording.is_none() {
            self.recording = Some(Vec::new());
        }
    }

    /// 기록 중인 frame이 `submission`으로 제출됨.
    pub fn frame_submitted(&mut self, submission: SubmissionId) {
        self.last_submitted = Some(submission);
        for item in self.recording.take().unwrap_or_default() {
            self.waiting.push_back((submission, item));
        }
    }

    /// 기록 중인 frame을 제출하지 않고 버림. 그 frame은 GPU에 닿지 않았으므로 마지막 submission
    /// 기준으로 돌아갑니다.
    pub fn frame_abandoned(&mut self) {
        for item in self.recording.take().unwrap_or_default() {
            self.after_last_submission(item);
        }
    }

    /// GPU가 `completed`까지 끝냈을 때 해제해도 되는 항목을 돌려줍니다.
    pub fn collect(&mut self, completed: Option<SubmissionId>) -> Vec<T> {
        if let Some(done) = completed {
            self.completed = Some(self.completed.map_or(done, |old| old.max(done)));
        }
        let mut released = std::mem::take(&mut self.ready);
        if let Some(done) = self.completed {
            while self
                .waiting
                .front()
                .is_some_and(|(submission, _)| *submission <= done)
            {
                if let Some((_, item)) = self.waiting.pop_front() {
                    released.push(item);
                }
            }
        }
        released
    }

    /// device lost: GPU 작업이 모두 사라졌으므로 모든 항목을 돌려주고 추적을 초기화합니다.
    pub fn drain_all(&mut self) -> Vec<T> {
        let mut released = std::mem::take(&mut self.ready);
        released.extend(self.waiting.drain(..).map(|(_, item)| item));
        released.extend(self.recording.take().unwrap_or_default());
        self.last_submitted = None;
        self.completed = None;
        released
    }
}

/// upload 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UploadTarget {
    /// buffer 범위.
    Buffer {
        /// buffer.
        buffer: BufferHandle,
        /// byte offset.
        offset: u64,
    },
    /// texture 영역.
    Texture {
        /// texture.
        texture: TextureHandle,
        /// 영역.
        region: TextureRegion,
        /// texture format(행 stride = width × pixel byte 수).
        format: TextureFormat,
    },
}

/// upload 요청 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRequest {
    /// 대상.
    pub target: UploadTarget,
    /// 데이터(공유 가능한 불변 bytes).
    pub data: Arc<[u8]>,
}

/// upload 요청 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UploadError {
    /// 빈 데이터.
    Empty,
    /// buffer offset + 길이가 넘칩니다.
    RangeOverflow,
    /// texture 영역이 0 크기입니다.
    EmptyRegion,
    /// upload할 수 없는 format(depth·stencil).
    UnsupportedFormat,
    /// 데이터 길이가 영역 크기 × pixel byte 수와 다릅니다.
    LengthMismatch {
        /// 기대 길이.
        expected: u64,
        /// 실제 길이.
        actual: u64,
    },
}

impl fmt::Display for UploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UploadError::Empty => f.write_str("빈 upload 데이터"),
            UploadError::RangeOverflow => f.write_str("upload 범위가 넘칩니다"),
            UploadError::EmptyRegion => f.write_str("0 크기 texture 영역"),
            UploadError::UnsupportedFormat => f.write_str("upload할 수 없는 texture format"),
            UploadError::LengthMismatch { expected, actual } => {
                write!(f, "upload 길이 불일치: 기대 {expected}, 실제 {actual}")
            }
        }
    }
}

impl std::error::Error for UploadError {}

/// upload 가능한 format의 pixel byte 수.
pub const fn upload_bytes_per_pixel(format: TextureFormat) -> Option<u64> {
    match format {
        TextureFormat::Rgba8Unorm
        | TextureFormat::Rgba8UnormSrgb
        | TextureFormat::Bgra8Unorm
        | TextureFormat::Bgra8UnormSrgb => Some(4),
        TextureFormat::Rgba16Float => Some(8),
        TextureFormat::R8Unorm => Some(1),
        TextureFormat::Depth32FloatStencil8 => None,
    }
}

/// flush 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct UploadFlush {
    /// 이번에 backend에 넘긴 요청 수.
    pub issued: usize,
    /// 이번에 넘긴 byte.
    pub bytes: u64,
    /// 남은 요청 수.
    pub remaining: usize,
}

/// FIFO upload queue.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UploadQueue {
    pending: VecDeque<UploadRequest>,
    pending_bytes: u64,
}

impl UploadQueue {
    /// 빈 queue.
    pub fn new() -> UploadQueue {
        UploadQueue::default()
    }

    /// 대기 요청 수.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// 대기 요청이 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// 대기 byte.
    pub const fn pending_bytes(&self) -> u64 {
        self.pending_bytes
    }

    /// 요청을 검증하고 queue 뒤에 넣습니다.
    pub fn push(&mut self, request: UploadRequest) -> Result<(), UploadError> {
        let len = request.data.len() as u64;
        if len == 0 {
            return Err(UploadError::Empty);
        }
        match request.target {
            UploadTarget::Buffer { offset, .. } => {
                offset.checked_add(len).ok_or(UploadError::RangeOverflow)?;
            }
            UploadTarget::Texture { region, format, .. } => {
                if region.width == 0 || region.height == 0 {
                    return Err(UploadError::EmptyRegion);
                }
                let bpp = upload_bytes_per_pixel(format).ok_or(UploadError::UnsupportedFormat)?;
                let expected = u64::from(region.width)
                    .checked_mul(u64::from(region.height))
                    .and_then(|pixels| pixels.checked_mul(bpp))
                    .ok_or(UploadError::RangeOverflow)?;
                if expected != len {
                    return Err(UploadError::LengthMismatch {
                        expected,
                        actual: len,
                    });
                }
            }
        }
        self.pending_bytes = self.pending_bytes.saturating_add(len);
        self.pending.push_back(request);
        Ok(())
    }

    /// 파괴할 buffer를 대상으로 한 대기 요청을 버립니다. 버린 수를 돌려줍니다.
    pub fn cancel_buffer(&mut self, buffer: BufferHandle) -> usize {
        self.cancel_where(
            |target| matches!(target, UploadTarget::Buffer { buffer: b, .. } if *b == buffer),
        )
    }

    /// 파괴할 texture를 대상으로 한 대기 요청을 버립니다. 버린 수를 돌려줍니다.
    pub fn cancel_texture(&mut self, texture: TextureHandle) -> usize {
        self.cancel_where(
            |target| matches!(target, UploadTarget::Texture { texture: t, .. } if *t == texture),
        )
    }

    fn cancel_where(&mut self, matches: impl Fn(&UploadTarget) -> bool) -> usize {
        let before = self.pending.len();
        let mut kept_bytes = 0u64;
        self.pending.retain(|request| {
            let keep = !matches(&request.target);
            if keep {
                kept_bytes = kept_bytes.saturating_add(request.data.len() as u64);
            }
            keep
        });
        self.pending_bytes = kept_bytes;
        before - self.pending.len()
    }

    /// 대기 요청을 FIFO로 `budget` byte까지 backend에 넘깁니다(frame encode 전 호출).
    ///
    /// 첫 요청은 예산보다 커도 넘겨 진행을 보장합니다. backend 오류가 나면 그 요청을 queue 앞에
    /// 남기고 오류를 돌려줍니다(재시도 가능).
    pub fn flush(
        &mut self,
        backend: &mut dyn RendererBackend,
        budget: u64,
    ) -> Result<UploadFlush, BackendError> {
        let mut result = UploadFlush::default();
        while let Some(request) = self.pending.front() {
            let len = request.data.len() as u64;
            if result.issued > 0 && result.bytes.saturating_add(len) > budget {
                break;
            }
            match request.target {
                UploadTarget::Buffer { buffer, offset } => {
                    backend.update_buffer(buffer, offset, &request.data)?
                }
                UploadTarget::Texture {
                    texture, region, ..
                } => backend.update_texture(texture, region, &request.data)?,
            }
            self.pending.pop_front();
            self.pending_bytes = self.pending_bytes.saturating_sub(len);
            result.issued += 1;
            result.bytes = result.bytes.saturating_add(len);
        }
        result.remaining = self.pending.len();
        Ok(result)
    }
}

#[cfg(test)]
mod tests;

//! Metal GPU timestamp query: pass별 GPU 시간 측정 (XI.8.6).
//!
//! - **지원 판정**: device가 stage boundary counter sampling
//!   (`supportsCounterSampling:MTLCounterSamplingPointAtStageBoundary`)을 지원하고 `timestamp` counter set이
//!   있어야 합니다. 지원하면 [`DeviceCapabilities::timestamp_query`]가 참이고,
//!   `DeviceConfig::timestamps`가 참일 때만 측정합니다.
//! - **기록**: frame마다 `MTLCounterSampleBuffer` 하나(pool 재사용)를 붙이고, graph pass `i`의 render pass
//!   descriptor에 vertex 시작(sample `2i`)·fragment 끝(`2i + 1`) timestamp를 기록합니다. pass가
//!   [`MAX_TIMED_PASSES`]개를 넘는 frame은 측정하지 않습니다.
//! - **해석**: submission이 GPU에서 끝나면(`poll`) sample을 resolve합니다. GPU tick → ns 환산은
//!   `sampleTimestamps:gpuTimestamp:`로 init 시점과 resolve 시점 CPU·GPU 시각 쌍을 잡아 비율을 구합니다
//!   (Apple Silicon에서는 보통 1:1). submission 전체 시간은 command buffer `GPUStartTime`·`GPUEndTime`
//!   입니다. 결과는 최근 [`MAX_TIMING_RESULTS`]개만 보관합니다.
//! - sample이 오류 값(`MTLCounterErrorValue`)이거나 끝이 시작보다 앞서면 그 pass는 0 ns로 보고합니다.
//!
//! [`DeviceCapabilities::timestamp_query`]: stellar_renderer_core::backend::DeviceCapabilities

use std::collections::VecDeque;
use std::fmt::Write as _;

use stellar_renderer_core::backend::{GpuTimings, PassGpuTime, SubmissionId};
use stellar_renderer_core::render_graph::PassId;

use crate::objc::{class, rust_string, sel, send, Id, NSUInteger, RawId};

/// 한 frame에서 측정하는 최대 pass 수.
pub const MAX_TIMED_PASSES: usize = 64;

/// 보관하는 submission timing 결과 최대 수.
pub const MAX_TIMING_RESULTS: usize = 64;

/// 동시에 쓰는 sample buffer 최대 수(넘는 frame은 측정하지 않음).
const MAX_SAMPLE_BUFFERS: usize = 8;

/// `MTLCounterSamplingPointAtStageBoundary`.
const SAMPLING_AT_STAGE_BOUNDARY: NSUInteger = 0;
/// `MTLCounterDontSample`.
const DONT_SAMPLE: NSUInteger = NSUInteger::MAX;
/// `MTLCounterErrorValue`.
const COUNTER_ERROR: u64 = u64::MAX;
/// `MTLStorageModeShared`.
const STORAGE_SHARED: NSUInteger = 0;

/// `NSRange`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct NSRange {
    location: NSUInteger,
    length: NSUInteger,
}

/// frame 하나의 sample 기록 정보.
#[derive(Debug)]
pub(crate) struct FrameTiming {
    buffer: Id,
    passes: Vec<PassId>,
}

impl FrameTiming {
    /// render pass descriptor에 붙일 (sample buffer, pass index).
    pub(crate) fn attachment(&self, pass_index: usize) -> (RawId, NSUInteger) {
        (self.buffer.as_raw(), pass_index as NSUInteger)
    }
}

/// render pass descriptor에 pass `index`의 timestamp sample을 붙입니다.
///
/// `descriptor`는 `MTLRenderPassDescriptor`이고 `buffer`는 2 × [`MAX_TIMED_PASSES`] sample 용량입니다.
pub(crate) fn attach_samples(descriptor: RawId, buffer: RawId, index: NSUInteger) {
    // SAFETY: `-sampleBufferAttachments`는 () -> array, `-objectAtIndexedSubscript:`(0)은
    // attachment descriptor, setter는 setSampleBuffer:(id)·set*SampleIndex:(NSUInteger)입니다.
    // index < MAX_TIMED_PASSES라 2·index + 1은 buffer 용량 안입니다.
    unsafe {
        let attachments: RawId = send(descriptor, sel!("sampleBufferAttachments"), ());
        let attachment: RawId = send(
            attachments,
            sel!("objectAtIndexedSubscript:"),
            (0 as NSUInteger,),
        );
        let _: () = send(attachment, sel!("setSampleBuffer:"), (buffer,));
        let _: () = send(
            attachment,
            sel!("setStartOfVertexSampleIndex:"),
            (2 * index,),
        );
        let _: () = send(
            attachment,
            sel!("setEndOfVertexSampleIndex:"),
            (DONT_SAMPLE,),
        );
        let _: () = send(
            attachment,
            sel!("setStartOfFragmentSampleIndex:"),
            (DONT_SAMPLE,),
        );
        let _: () = send(
            attachment,
            sel!("setEndOfFragmentSampleIndex:"),
            (2 * index + 1,),
        );
    }
}

/// CPU·GPU 시각 쌍.
fn sample_clocks(device: &Id) -> (u64, u64) {
    let (mut cpu, mut gpu) = (0u64, 0u64);
    // SAFETY: `-sampleTimestamps:gpuTimestamp:`는 (MTLTimestamp*, MTLTimestamp*) -> void입니다.
    unsafe {
        let _: () = device.send(
            sel!("sampleTimestamps:gpuTimestamp:"),
            (&mut cpu as *mut u64, &mut gpu as *mut u64),
        );
    }
    (cpu, gpu)
}

/// device가 stage boundary timestamp를 지원하면 `timestamp` counter set을 돌려줍니다.
pub(crate) fn timestamp_counter_set(device: &Id) -> Option<Id> {
    // SAFETY: `-supportsCounterSampling:`은 (NSUInteger) -> BOOL, `-counterSets`는 () -> NSArray*,
    // NSArray `-count`·`-objectAtIndex:`, counter set `-name`은 () -> NSString*입니다.
    unsafe {
        let supported: bool = device.send(
            sel!("supportsCounterSampling:"),
            (SAMPLING_AT_STAGE_BOUNDARY,),
        );
        if !supported {
            return None;
        }
        let sets: RawId = device.send(sel!("counterSets"), ());
        if sets.is_null() {
            return None;
        }
        let count: NSUInteger = send(sets, sel!("count"), ());
        (0..count).find_map(|index| {
            let set: RawId = send(sets, sel!("objectAtIndex:"), (index,));
            let name: RawId = send(set, sel!("name"), ());
            (rust_string(name, 64).as_deref() == Some("timestamp"))
                .then(|| Id::from_borrowed(set))
                .flatten()
        })
    }
}

/// GPU timestamp 측정기.
#[derive(Debug)]
pub(crate) struct Timestamps {
    counter_set: Id,
    calibration: (u64, u64),
    free: Vec<Id>,
    live: usize,
    results: VecDeque<(SubmissionId, GpuTimings)>,
}

impl Timestamps {
    /// 측정기를 만듭니다(counter set은 [`timestamp_counter_set`] 결과).
    pub(crate) fn new(device: &Id, counter_set: Id) -> Timestamps {
        Timestamps {
            counter_set,
            calibration: sample_clocks(device),
            free: Vec::new(),
            live: 0,
            results: VecDeque::new(),
        }
    }

    /// frame 하나를 측정할 sample buffer를 준비합니다. 측정할 수 없으면 `None`(frame은 그대로 진행).
    pub(crate) fn begin_frame(&mut self, device: &Id, passes: &[PassId]) -> Option<FrameTiming> {
        if passes.is_empty() || passes.len() > MAX_TIMED_PASSES {
            return None;
        }
        let buffer = match self.free.pop() {
            Some(buffer) => buffer,
            None if self.live < MAX_SAMPLE_BUFFERS => {
                let buffer = self.create_buffer(device)?;
                self.live += 1;
                buffer
            }
            None => return None,
        };
        Some(FrameTiming {
            buffer,
            passes: passes.to_vec(),
        })
    }

    /// 쓰지 않은 frame timing을 돌려받습니다(discard·encode 실패).
    pub(crate) fn recycle(&mut self, timing: FrameTiming) {
        self.free.push(timing.buffer);
    }

    fn create_buffer(&self, device: &Id) -> Option<Id> {
        let class = class!("MTLCounterSampleBufferDescriptor")?;
        // SAFETY: `+new`는 () -> id(+1), descriptor setter: setCounterSet:(id),
        // setStorageMode:/setSampleCount:(NSUInteger). `-newCounterSampleBufferWithDescriptor:error:`는
        // (descriptor, NSError**) -> id(+1, 실패 시 nil, error는 +0)입니다.
        unsafe {
            let descriptor = Id::from_owned(send(class.as_raw(), sel!("new"), ()))?;
            let _: () = descriptor.send(sel!("setCounterSet:"), (self.counter_set.as_raw(),));
            let _: () = descriptor.send(sel!("setStorageMode:"), (STORAGE_SHARED,));
            let _: () = descriptor.send(
                sel!("setSampleCount:"),
                ((2 * MAX_TIMED_PASSES) as NSUInteger,),
            );
            let mut error: RawId = std::ptr::null_mut();
            Id::from_owned(device.send(
                sel!("newCounterSampleBufferWithDescriptor:error:"),
                (descriptor.as_raw(), &mut error as *mut RawId),
            ))
        }
    }

    /// GPU에서 끝난 submission의 sample을 해석해 보관하고 buffer를 pool에 돌려줍니다.
    pub(crate) fn complete(
        &mut self,
        device: &Id,
        submission: SubmissionId,
        command_buffer: &Id,
        timing: FrameTiming,
    ) {
        let ticks = resolve(&timing.buffer, timing.passes.len());
        let (cpu_now, gpu_now) = sample_clocks(device);
        let (cpu_then, gpu_then) = self.calibration;
        let gpu_span = gpu_now.saturating_sub(gpu_then);
        let ratio = if gpu_span == 0 {
            1.0
        } else {
            cpu_now.saturating_sub(cpu_then) as f64 / gpu_span as f64
        };
        let passes = timing
            .passes
            .iter()
            .enumerate()
            .map(|(index, &pass)| {
                let nanos = ticks
                    .as_ref()
                    .and_then(|ticks| {
                        let (start, end) = (*ticks.get(2 * index)?, *ticks.get(2 * index + 1)?);
                        (start != COUNTER_ERROR && end != COUNTER_ERROR && end >= start)
                            .then(|| ((end - start) as f64 * ratio).round() as u64)
                    })
                    .unwrap_or(0);
                PassGpuTime { pass, nanos }
            })
            .collect();
        // SAFETY: `-GPUStartTime`·`-GPUEndTime`은 () -> CFTimeInterval(double, 초)입니다.
        let (start, end): (f64, f64) = unsafe {
            (
                command_buffer.send(sel!("GPUStartTime"), ()),
                command_buffer.send(sel!("GPUEndTime"), ()),
            )
        };
        let total_nanos = if end > start {
            ((end - start) * 1e9).round() as u64
        } else {
            0
        };
        if self.results.len() == MAX_TIMING_RESULTS {
            self.results.pop_front();
        }
        self.results.push_back((
            submission,
            GpuTimings {
                passes,
                total_nanos,
            },
        ));
        self.free.push(timing.buffer);
    }

    /// 보관한 결과.
    pub(crate) fn result(&self, submission: SubmissionId) -> Option<&GpuTimings> {
        self.results
            .iter()
            .find(|(id, _)| *id == submission)
            .map(|(_, timings)| timings)
    }
}

/// sample buffer 앞 `2 × passes` sample(timestamp tick).
fn resolve(buffer: &Id, passes: usize) -> Option<Vec<u64>> {
    let range = NSRange {
        location: 0,
        length: 2 * passes,
    };
    // SAFETY: `-resolveCounterRange:`는 (NSRange) -> NSData*(+0, 실패 시 nil)이고, NSData
    // `-bytes`·`-length`는 () -> const void*·NSUInteger입니다. timestamp sample은 u64 하나씩입니다.
    unsafe {
        let data: RawId = buffer.send(sel!("resolveCounterRange:"), (range,));
        if data.is_null() {
            return None;
        }
        let bytes: *const u8 = send(data, sel!("bytes"), ());
        let length: NSUInteger = send(data, sel!("length"), ());
        if bytes.is_null() {
            return None;
        }
        let count = (length / 8).min(2 * passes);
        Some(
            (0..count)
                .map(|index| std::ptr::read_unaligned(bytes.add(index * 8).cast::<u64>()))
                .collect(),
        )
    }
}

/// GPU 시간 보고 한 줄(`renderer.metal.gpu submission=.. total_ns=.. pass<id>_ns=..`).
pub fn gpu_timing_line(submission: SubmissionId, timings: &GpuTimings) -> String {
    let mut line = format!(
        "renderer.metal.gpu submission={} total_ns={}",
        submission.0, timings.total_nanos
    );
    for pass in &timings.passes {
        // String에 쓰기는 실패하지 않습니다.
        let _ = write!(line, " pass{}_ns={}", pass.pass.0, pass.nanos);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_line_is_fixed_format() {
        let timings = GpuTimings {
            passes: vec![
                PassGpuTime {
                    pass: PassId(1),
                    nanos: 1_500,
                },
                PassGpuTime {
                    pass: PassId(0),
                    nanos: 2_000,
                },
            ],
            total_nanos: 4_000,
        };
        assert_eq!(
            gpu_timing_line(SubmissionId(3), &timings),
            "renderer.metal.gpu submission=3 total_ns=4000 pass1_ns=1500 pass0_ns=2000"
        );
    }
}

//! upload queue·frame-local allocator 테스트 (XI.5.3).

use std::collections::VecDeque;

use super::*;
use crate::backend::mock::MockBackend;
use crate::backend::tests::device_config;
use crate::backend::{
    BackendCall, BackendErrorKind, BufferDesc, BufferUsage, CheckedBackend, GpuTextureDesc,
    TextureUsage,
};
use crate::gpu_handle::{HandleTable, TextureKind};
use crate::render_graph::SampleCount;

/// 결정론적 xorshift.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

/// GPU에 제출된 frame이 끝날 때 확인할 내용.
struct SimFrame {
    submission: SubmissionId,
    /// (slot, offset, 기대 bytes): GPU가 끝날 때 slot 메모리가 여전히 이 값이어야 합니다.
    reads: Vec<(u8, u64, Vec<u8>)>,
    /// GPU가 끝날 때 살아 있어야 하는 texture.
    textures: Vec<TextureHandle>,
}

/// 느린 GPU 모사: slot 메모리와 texture table을 GPU가 끝날 때 검사합니다.
struct SimGpu {
    memory: Vec<Vec<u8>>,
    textures: HandleTable<TextureKind, u64>,
    queue: VecDeque<SimFrame>,
    next_submission: u64,
    completed: Option<SubmissionId>,
    premature_frees: usize,
    overwritten_reads: usize,
    verified_frames: usize,
}

impl SimGpu {
    fn new(slots: u8, capacity: u64) -> SimGpu {
        SimGpu {
            memory: vec![vec![0; capacity as usize]; usize::from(slots)],
            textures: HandleTable::new(0),
            queue: VecDeque::new(),
            next_submission: 1,
            completed: None,
            premature_frees: 0,
            overwritten_reads: 0,
            verified_frames: 0,
        }
    }

    fn submit(
        &mut self,
        reads: Vec<(u8, u64, Vec<u8>)>,
        textures: Vec<TextureHandle>,
    ) -> SubmissionId {
        let submission = SubmissionId(self.next_submission);
        self.next_submission += 1;
        self.queue.push_back(SimFrame {
            submission,
            reads,
            textures,
        });
        submission
    }

    /// 가장 오래된 frame을 끝내며 그 frame이 읽는 메모리·texture를 검사합니다.
    fn complete_next(&mut self) -> bool {
        let Some(frame) = self.queue.pop_front() else {
            return false;
        };
        for (slot, offset, expected) in &frame.reads {
            let start = *offset as usize;
            let actual = &self.memory[usize::from(*slot)][start..start + expected.len()];
            if actual != expected.as_slice() {
                self.overwritten_reads += 1;
            }
        }
        for texture in &frame.textures {
            if !self.textures.contains(*texture) {
                self.premature_frees += 1;
            }
        }
        self.completed = Some(frame.submission);
        self.verified_frames += 1;
        true
    }
}

#[test]
fn three_frames_in_flight_never_free_early() {
    // 검증 기준(XI.5.3): 3 frames-in-flight simulation에서 premature free 없음.
    const CAPACITY: u64 = 2048;
    let mut rng = Rng(0x5eed_f00d_1234_5678);
    let mut allocator = RingFrameAllocator::new(3, CAPACITY).unwrap();
    let mut release: DeferredRelease<TextureHandle> = DeferredRelease::new();
    let mut gpu = SimGpu::new(3, CAPACITY);
    let mut live: Vec<TextureHandle> = (0..8).map(|n| gpu.textures.insert(n).unwrap()).collect();
    let mut released_total = 0usize;
    let mut deferred_total = 0usize;
    let mut stalls = 0usize;
    let mut abandoned = 0usize;
    let mut max_in_flight = 0usize;

    for frame_number in 1..=400u64 {
        // GPU 진행을 반영해 slot·resource를 회수합니다.
        let retire = |allocator: &mut RingFrameAllocator,
                      release: &mut DeferredRelease<TextureHandle>,
                      gpu: &mut SimGpu,
                      released_total: &mut usize| {
            if let Some(done) = gpu.completed {
                allocator.retire(done);
            }
            for texture in release.collect(gpu.completed) {
                gpu.textures.remove(texture).unwrap();
                *released_total += 1;
            }
        };
        retire(&mut allocator, &mut release, &mut gpu, &mut released_total);

        // 다음 slot이 아직 GPU에 있으면 GPU를 기다립니다(frame pacing).
        loop {
            match allocator.begin_frame(FrameId(frame_number)) {
                Ok(_) => break,
                Err(FrameAllocError::SlotBusy { submission, .. }) => {
                    stalls += 1;
                    assert!(gpu
                        .queue
                        .front()
                        .is_some_and(|f| f.submission <= submission));
                    assert!(gpu.complete_next());
                    retire(&mut allocator, &mut release, &mut gpu, &mut released_total);
                }
                Err(other) => panic!("예상하지 못한 오류: {other}"),
            }
        }
        release.frame_begun();

        // frame-local 데이터를 할당하고 frame 고유 bytes를 씁니다.
        let mut reads = Vec::new();
        for _ in 0..rng.below(6) + 1 {
            let size = rng.below(200) + 1;
            let align = 1u64 << rng.below(5);
            let Ok(allocation) = allocator.allocate(size, align) else {
                break;
            };
            assert_eq!(allocation.offset % align, 0);
            let bytes: Vec<u8> = (0..size)
                .map(|i| (frame_number as u8) ^ (i as u8).wrapping_mul(31))
                .collect();
            let start = allocation.offset as usize;
            gpu.memory[usize::from(allocation.slot)][start..start + bytes.len()]
                .copy_from_slice(&bytes);
            reads.push((allocation.slot, allocation.offset, bytes));
        }

        // 살아 있는 texture 일부를 참조하고, 가끔 하나를 파괴 요청합니다(참조 중일 수 있음).
        let mut referenced = Vec::new();
        for texture in &live {
            if rng.below(3) == 0 {
                referenced.push(*texture);
            }
        }
        if rng.below(4) == 0 && !live.is_empty() {
            let index = rng.below(live.len() as u64) as usize;
            release.defer(live.swap_remove(index));
            deferred_total += 1;
        }
        if rng.below(3) == 0 {
            live.push(gpu.textures.insert(frame_number).unwrap());
        }

        if rng.below(10) == 0 {
            // 제출하지 않고 버린 frame(stale drop 등): GPU는 이 frame을 보지 않습니다.
            allocator.abandon_frame().unwrap();
            release.frame_abandoned();
            abandoned += 1;
        } else {
            let submission = gpu.submit(reads, referenced);
            allocator.end_frame(submission).unwrap();
            release.frame_submitted(submission);
        }
        max_in_flight = max_in_flight.max(allocator.in_flight());

        // 느린 GPU: frame마다 0~1개만 끝냅니다(평균 0.5개 → 계속 밀림).
        if rng.below(2) == 0 {
            gpu.complete_next();
        }
    }
    while gpu.complete_next() {}
    if let Some(done) = gpu.completed {
        allocator.retire(done);
    }
    for texture in release.collect(gpu.completed) {
        gpu.textures.remove(texture).unwrap();
        released_total += 1;
    }

    assert_eq!(gpu.premature_frees, 0, "GPU가 쓰는 중 해제된 texture");
    assert_eq!(
        gpu.overwritten_reads, 0,
        "GPU가 읽기 전 덮어쓴 frame 메모리"
    );
    assert!(stalls > 0, "느린 GPU에서는 slot 대기가 생겨야 합니다");
    assert!(abandoned > 0);
    assert_eq!(max_in_flight, 3);
    assert!(gpu.verified_frames > 300);
    assert_eq!(released_total, deferred_total);
    assert_eq!(release.pending(), 0);
    assert_eq!(allocator.in_flight(), 0);
}

#[test]
fn ring_slots_reopen_only_after_retire() {
    let mut allocator = RingFrameAllocator::new(2, 64).unwrap();
    assert_eq!(allocator.slot_count(), 2);
    assert_eq!(allocator.capacity(), 64);
    assert_eq!(allocator.begin_frame(FrameId(1)), Ok(0));
    assert_eq!(
        allocator.begin_frame(FrameId(2)),
        Err(FrameAllocError::FrameAlreadyOpen)
    );
    let a = allocator.allocate(10, 1).unwrap();
    let b = allocator.allocate(4, 16).unwrap();
    assert_eq!((a.offset, b.offset), (0, 16));
    assert_eq!(allocator.used(), 20);
    assert_eq!(
        allocator.allocate(64, 1),
        Err(FrameAllocError::OutOfSpace {
            requested: 64,
            remaining: 44
        })
    );
    allocator.end_frame(SubmissionId(1)).unwrap();
    assert_eq!(allocator.allocate(1, 1), Err(FrameAllocError::NoOpenFrame));
    assert_eq!(allocator.begin_frame(FrameId(2)), Ok(1));
    allocator.end_frame(SubmissionId(2)).unwrap();
    // 두 slot 모두 GPU에 있으므로 slot 0은 submission 1이 끝날 때까지 열리지 않습니다.
    assert_eq!(
        allocator.begin_frame(FrameId(3)),
        Err(FrameAllocError::SlotBusy {
            slot: 0,
            submission: SubmissionId(1)
        })
    );
    allocator.retire(SubmissionId(1));
    assert_eq!(allocator.in_flight(), 1);
    assert_eq!(allocator.begin_frame(FrameId(3)), Ok(0));
    // 버린 frame의 slot은 바로 다음 frame이 다시 씁니다.
    allocator.abandon_frame().unwrap();
    assert_eq!(allocator.begin_frame(FrameId(4)), Ok(0));
    assert_eq!(allocator.abandon_frame(), Ok(()));
    assert_eq!(allocator.abandon_frame(), Err(FrameAllocError::NoOpenFrame));
    assert_eq!(
        allocator.end_frame(SubmissionId(9)),
        Err(FrameAllocError::NoOpenFrame)
    );
    allocator.reset();
    assert_eq!(allocator.in_flight(), 0);
    assert_eq!(allocator.begin_frame(FrameId(5)), Ok(0));
}

#[test]
fn allocator_rejects_invalid_requests() {
    assert_eq!(
        RingFrameAllocator::new(0, 64),
        Err(FrameAllocError::InvalidFramesInFlight { requested: 0 })
    );
    assert_eq!(
        RingFrameAllocator::new(4, 64),
        Err(FrameAllocError::InvalidFramesInFlight { requested: 4 })
    );
    assert_eq!(
        RingFrameAllocator::new(3, 0),
        Err(FrameAllocError::InvalidCapacity)
    );
    let mut allocator = RingFrameAllocator::new(1, u64::MAX).unwrap();
    allocator.begin_frame(FrameId(1)).unwrap();
    assert_eq!(allocator.allocate(0, 4), Err(FrameAllocError::ZeroSize));
    assert_eq!(
        allocator.allocate(4, 3),
        Err(FrameAllocError::InvalidAlignment { align: 3 })
    );
    assert_eq!(
        allocator.allocate(4, 512),
        Err(FrameAllocError::InvalidAlignment { align: 512 })
    );
    // offset 계산이 넘쳐도 panic 없이 거부합니다.
    allocator.allocate(u64::MAX - 2, 1).unwrap();
    assert!(matches!(
        allocator.allocate(1, 8),
        Err(FrameAllocError::OutOfSpace { .. })
    ));
    assert!(matches!(
        allocator.allocate(4, 1),
        Err(FrameAllocError::OutOfSpace { .. })
    ));
    for error in [
        FrameAllocError::ZeroSize,
        FrameAllocError::InvalidCapacity,
        FrameAllocError::SlotBusy {
            slot: 1,
            submission: SubmissionId(2),
        },
    ] {
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn deferred_release_waits_for_the_last_possible_use() {
    let mut release: DeferredRelease<&str> = DeferredRelease::new();
    // 아무 frame도 제출하지 않았으면 바로 해제됩니다.
    release.defer("idle");
    assert_eq!(release.collect(None), vec!["idle"]);

    // frame 1 기록 중 요청: frame 1 submission까지 대기.
    release.frame_begun();
    release.defer("in-recording");
    assert!(release.collect(None).is_empty());
    release.frame_submitted(SubmissionId(1));
    assert!(release.collect(None).is_empty());

    // frame 사이 요청: 마지막 submission(1)까지 대기.
    release.defer("between");
    release.frame_begun();
    release.defer("frame-2");
    release.frame_submitted(SubmissionId(2));
    assert_eq!(release.pending(), 3);
    assert_eq!(
        release.collect(Some(SubmissionId(1))),
        vec!["in-recording", "between"]
    );
    // 버린 frame에서 요청: GPU가 그 frame을 보지 않았으므로 마지막 submission(2) 기준.
    release.frame_begun();
    release.defer("abandoned");
    release.frame_abandoned();
    assert!(release.collect(Some(SubmissionId(1))).is_empty());
    assert_eq!(
        release.collect(Some(SubmissionId(2))),
        vec!["frame-2", "abandoned"]
    );
    // 이미 끝난 submission 뒤의 요청은 바로 해제됩니다.
    release.defer("after-complete");
    assert_eq!(release.collect(None), vec!["after-complete"]);
    // 완료 번호는 줄어들지 않습니다.
    release.frame_begun();
    release.frame_submitted(SubmissionId(3));
    release.defer("late");
    assert!(release.collect(Some(SubmissionId(1))).is_empty());
    assert_eq!(release.pending(), 1);
    // device lost: 모두 돌려받습니다.
    release.frame_begun();
    release.defer("recording");
    let mut drained = release.drain_all();
    drained.sort_unstable();
    assert_eq!(drained, vec!["late", "recording"]);
    assert_eq!(release.pending(), 0);
}

fn texture_desc() -> GpuTextureDesc {
    GpuTextureDesc {
        width: 4,
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

fn region(width: u32, height: u32) -> TextureRegion {
    TextureRegion {
        mip_level: 0,
        x: 0,
        y: 0,
        width,
        height,
    }
}

#[test]
fn upload_queue_flushes_fifo_within_budget() {
    let mut backend = CheckedBackend::new(MockBackend::new());
    backend.init(&device_config(8, 8)).unwrap();
    let texture = backend.create_texture(&texture_desc()).unwrap();
    let buffer = backend
        .create_buffer(&BufferDesc {
            size: 1024,
            usage: BufferUsage::Vertex,
        })
        .unwrap();
    let calls_before = backend.inner().calls.len();

    let mut queue = UploadQueue::new();
    queue
        .push(UploadRequest {
            target: UploadTarget::Texture {
                texture,
                region: region(4, 2),
                format: TextureFormat::Rgba8Unorm,
            },
            data: Arc::from(vec![1u8; 32]),
        })
        .unwrap();
    for _ in 0..3 {
        queue
            .push(UploadRequest {
                target: UploadTarget::Buffer { buffer, offset: 0 },
                data: Arc::from(vec![2u8; 100]),
            })
            .unwrap();
    }
    assert_eq!(queue.len(), 4);
    assert_eq!(queue.pending_bytes(), 332);

    // 예산 150: texture(32) + buffer(100) = 132, 다음 100은 넘으므로 다음 frame으로.
    let first = queue.flush(&mut backend, 150).unwrap();
    assert_eq!(
        first,
        UploadFlush {
            issued: 2,
            bytes: 132,
            remaining: 2
        }
    );
    // 예산보다 큰 요청도 하나는 넘겨 진행합니다.
    let second = queue.flush(&mut backend, 10).unwrap();
    assert_eq!(second.issued, 1);
    assert_eq!(second.bytes, 100);
    let third = queue.flush(&mut backend, 1_000).unwrap();
    assert_eq!(third.remaining, 0);
    assert!(queue.is_empty());
    assert_eq!(queue.pending_bytes(), 0);
    assert_eq!(
        backend.inner().calls[calls_before..],
        [
            BackendCall::UpdateTexture,
            BackendCall::UpdateBuffer,
            BackendCall::UpdateBuffer,
            BackendCall::UpdateBuffer,
        ]
    );
}

#[test]
fn upload_queue_validates_cancels_and_retries() {
    let mut backend = CheckedBackend::new(MockBackend::new());
    backend.init(&device_config(8, 8)).unwrap();
    let texture = backend.create_texture(&texture_desc()).unwrap();
    let other = backend.create_texture(&texture_desc()).unwrap();
    let buffer = backend
        .create_buffer(&BufferDesc {
            size: 64,
            usage: BufferUsage::Uniform,
        })
        .unwrap();
    let mut queue = UploadQueue::new();
    let tex = |texture, w, h, format, len: usize| UploadRequest {
        target: UploadTarget::Texture {
            texture,
            region: region(w, h),
            format,
        },
        data: Arc::from(vec![0u8; len]),
    };
    assert_eq!(
        queue.push(tex(texture, 4, 2, TextureFormat::Rgba8Unorm, 0)),
        Err(UploadError::Empty)
    );
    assert_eq!(
        queue.push(tex(texture, 0, 2, TextureFormat::Rgba8Unorm, 8)),
        Err(UploadError::EmptyRegion)
    );
    assert_eq!(
        queue.push(tex(texture, 4, 2, TextureFormat::Depth32FloatStencil8, 8)),
        Err(UploadError::UnsupportedFormat)
    );
    assert_eq!(
        queue.push(tex(texture, 4, 2, TextureFormat::Rgba8Unorm, 31)),
        Err(UploadError::LengthMismatch {
            expected: 32,
            actual: 31
        })
    );
    assert_eq!(
        queue.push(UploadRequest {
            target: UploadTarget::Buffer {
                buffer,
                offset: u64::MAX
            },
            data: Arc::from(vec![0u8; 2]),
        }),
        Err(UploadError::RangeOverflow)
    );
    assert!(queue.is_empty());

    queue
        .push(tex(texture, 4, 2, TextureFormat::R8Unorm, 8))
        .unwrap();
    queue
        .push(tex(other, 4, 2, TextureFormat::Rgba8Unorm, 32))
        .unwrap();
    queue
        .push(tex(texture, 2, 2, TextureFormat::Rgba16Float, 32))
        .unwrap();
    queue
        .push(UploadRequest {
            target: UploadTarget::Buffer { buffer, offset: 8 },
            data: Arc::from(vec![0u8; 8]),
        })
        .unwrap();
    // 파괴할 texture의 대기 upload를 버립니다(파괴 뒤 stale handle upload 방지).
    assert_eq!(queue.cancel_texture(texture), 2);
    assert_eq!(queue.len(), 2);
    assert_eq!(queue.pending_bytes(), 40);
    assert_eq!(queue.cancel_buffer(buffer), 1);
    assert_eq!(queue.pending_bytes(), 32);

    // backend 오류가 나면 요청을 남겨 다음 flush에서 재시도합니다.
    backend.inner_mut().fail_next = Some((
        BackendCall::UpdateTexture,
        BackendErrorKind::OutOfDeviceMemory,
    ));
    let error = queue.flush(&mut backend, 1_000).unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::OutOfDeviceMemory);
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.flush(&mut backend, 1_000).unwrap().issued, 1);
    for error in [
        UploadError::Empty,
        UploadError::RangeOverflow,
        UploadError::LengthMismatch {
            expected: 1,
            actual: 2,
        },
    ] {
        assert!(!error.to_string().is_empty());
    }
}

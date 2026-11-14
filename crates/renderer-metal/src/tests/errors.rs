//! XI.8.7: device error·memory pressure·drawable timeout 처리 — 앱 crash 없이 error event 발생.
//!
//! - GPU 실행 오류: 실제 GPU fault를 일부러 내면 test process 전체의 GPU 접근이 막힐 수 있어(Metal
//!   `AccessRevoked`), GPU에서 정상 완료한 submission을 `MTLCommandBufferError` 코드로 실패한 것처럼 보는
//!   테스트 주입(`injected_gpu_errors`)을 씁니다. 그 뒤 분류·event·device lost 상태·재 init 복구는 실제
//!   device로 확인합니다.
//! - memory pressure: 예산을 작게 바꿔 실제 할당 경로에서 거부·event·회수를 확인합니다. device 상한을 넘는
//!   buffer는 실제로 Metal 할당 전에 거부됩니다.
//! - drawable timeout: 창 밖 layer는 drawable이 고갈되지 않아(XI.8.2) nil drawable을 주입합니다.

use std::time::{Duration, Instant};

use stellar_renderer_core::backend::recovery::{RecoveryAction, RecoveryTracker};
use stellar_renderer_core::backend::{
    BackendError, BackendErrorKind, BlendState, BufferDesc, BufferUsage, CallOrderState,
    CheckedBackend, DrawCommand, EncodeOutcome, RendererBackend, SkipReason, SubmissionId,
    TextureHandle,
};
use stellar_renderer_core::render_graph::{
    AttachmentSize, CompiledRenderGraph, ImportKind, LoadOp, PassKind, RenderGraphBuilder,
    SampleCount, StoreOp, TextureDesc, TextureFormat,
};

use super::geometry::{draw_uniforms, quad, single_pass, solid_key, Offscreen};
use super::*;
use crate::{LayerOptions, MetalBackend, MetalEvent, ShaderErrorStage, MAX_EVENTS};

const MIB: u64 = 1024 * 1024;

/// external RGBA8 target에 clear frame 하나를 제출합니다.
fn submit_clear(
    backend: &mut CheckedBackend<MetalBackend>,
    frame: u64,
    target: TextureHandle,
    size: u32,
) -> SubmissionId {
    let graph = external_clear_graph(size, size, TextureFormat::Rgba8Unorm, unit_clear([true; 4]));
    let imports = bind_first_import(&graph, target);
    let EncodeOutcome::Encoded(encoded) = backend
        .encode(&request(frame, &graph, &imports))
        .expect("encode")
    else {
        panic!("offscreen frame은 skip하지 않습니다");
    };
    backend.submit(encoded).expect("submit")
}

/// GPU 결과가 나올 때까지 `poll`합니다(오류면 그 오류).
fn poll_until_done(
    backend: &mut CheckedBackend<MetalBackend>,
    submission: SubmissionId,
) -> Result<(), BackendError> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let progress = backend.poll()?;
        if progress.completed >= Some(submission) {
            return Ok(());
        }
        assert!(
            Instant::now() < deadline,
            "GPU가 10초 안에 끝나지 않았습니다"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn headless(size: u32) -> (CheckedBackend<MetalBackend>, TextureHandle) {
    let mut backend = CheckedBackend::new(MetalBackend::headless());
    backend
        .init(&device_config(size, size, TextureFormat::Bgra8Unorm))
        .expect("init");
    let target = backend
        .create_texture(&target_desc(size, size, TextureFormat::Rgba8Unorm))
        .expect("target");
    (backend, target)
}

#[test]
fn device_removed_reports_event_and_recovers_by_reinit() {
    // 검증 기준(XI.8.7): device error에서 앱 crash 없이 error event 발생, 재 init으로 복구.
    if !metal_available() {
        return;
    }
    let (mut backend, target) = headless(4);
    let first = submit_clear(&mut backend, 1, target, 4);
    poll_until_done(&mut backend, first).expect("정상 frame");
    backend.inner_mut().injected_gpu_errors.push_back(11);
    let failed = submit_clear(&mut backend, 2, target, 4);
    let error = poll_until_done(&mut backend, failed).unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::DeviceLost);
    assert_eq!(
        backend.inner_mut().drain_events(),
        vec![MetalEvent::DeviceError {
            kind: BackendErrorKind::DeviceLost,
            submission: Some(failed),
            command_buffer_error: Some(11),
        }]
    );
    // 공통 계층 복구: device 재생성. 그 전까지는 init만 허용됩니다.
    let mut tracker = RecoveryTracker::new();
    assert_eq!(tracker.on_error(&error), RecoveryAction::RecreateDevice);
    assert_eq!(backend.validator().state(), CallOrderState::Lost);
    assert!(backend
        .create_texture(&target_desc(4, 4, TextureFormat::Rgba8Unorm))
        .is_err());
    // raw backend도 lost 동안 init 아닌 호출을 DeviceLost로 거부합니다.
    assert_eq!(
        backend.inner_mut().read_texture(target).unwrap_err().kind(),
        BackendErrorKind::DeviceLost
    );
    backend
        .init(&device_config(4, 4, TextureFormat::Bgra8Unorm))
        .expect("re-init");
    let fresh = backend
        .create_texture(&target_desc(4, 4, TextureFormat::Rgba8Unorm))
        .expect("new target");
    let submission = submit_clear(&mut backend, 3, fresh, 4);
    assert_eq!(
        submission,
        SubmissionId(1),
        "새 device는 submission을 다시 셉니다"
    );
    poll_until_done(&mut backend, submission).expect("복구 뒤 frame");
    tracker.frame_succeeded();
    assert!(all_pixels(
        &backend.inner_mut().read_texture(fresh).expect("readback"),
        [255; 4]
    ));
    // 이전 device의 handle은 무효입니다.
    assert_eq!(
        backend.destroy_texture(target).unwrap_err().kind(),
        BackendErrorKind::InvalidUsage
    );
}

#[test]
fn transient_gpu_errors_keep_the_device_usable() {
    if !metal_available() {
        return;
    }
    let (mut backend, target) = headless(4);
    // timeout(2) → 다음 frame 재시도, out-of-memory(8) → cache purge 후 재시도, page fault(3) → frame skip.
    for (frame, (code, kind, action)) in (1..).step_by(2).zip([
        (2, BackendErrorKind::Timeout, RecoveryAction::RetryNextFrame),
        (
            8,
            BackendErrorKind::OutOfDeviceMemory,
            RecoveryAction::PurgeCachesAndRetry,
        ),
        (3, BackendErrorKind::InvalidUsage, RecoveryAction::SkipFrame),
    ]) {
        backend.inner_mut().injected_gpu_errors.push_back(code);
        let failed = submit_clear(&mut backend, frame, target, 4);
        let error = poll_until_done(&mut backend, failed).unwrap_err();
        assert_eq!(error.kind(), kind, "code {code}");
        assert_eq!(RecoveryTracker::new().on_error(&error), action);
        let events = backend.inner_mut().drain_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), kind);
        assert!(events[0]
            .to_line()
            .contains(&format!("command_buffer_error={code}")));
        // device는 그대로 쓸 수 있습니다.
        assert_eq!(backend.validator().state(), CallOrderState::Idle);
        let next = submit_clear(&mut backend, frame + 1, target, 4);
        poll_until_done(&mut backend, next).expect("다음 frame");
    }
    assert!(all_pixels(
        &backend.inner_mut().read_texture(target).expect("readback"),
        [255; 4]
    ));
}

/// `size`² RGBA8 transient 하나를 거쳐 target을 지우는 graph.
fn transient_graph(size: u32) -> CompiledRenderGraph {
    let fixed = TextureDesc {
        format: TextureFormat::Rgba8Unorm,
        size: AttachmentSize::Fixed {
            width: size,
            height: size,
        },
        samples: SampleCount::X1,
    };
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import("target", fixed, ImportKind::External);
    let layer = builder.transient("layer", fixed);
    let clear = unit_clear([false, false, false, true]);
    let paint = builder.pass("layer", PassKind::Raster);
    let layer_v1 = builder
        .color(paint, layer, LoadOp::Clear, clear, StoreOp::Store)
        .expect("layer");
    let composite = builder.pass("composite", PassKind::Raster);
    builder.read(composite, layer_v1).expect("read");
    let written = builder
        .color(composite, target, LoadOp::Clear, clear, StoreOp::Store)
        .expect("target");
    builder.set_output(written);
    builder.finish().compile().expect("compile")
}

#[test]
fn memory_pressure_rejects_allocations_with_events() {
    // 검증 기준(XI.8.7): memory pressure에서 crash 없이 error event.
    if !metal_available() {
        return;
    }
    let mut backend = CheckedBackend::new(MetalBackend::headless());
    backend
        .init(&device_config(64, 64, TextureFormat::Bgra8Unorm))
        .expect("init");
    let default_budget = backend.inner().memory_usage().budget_bytes;
    assert!(
        default_budget > 0,
        "기본 예산은 recommendedMaxWorkingSetSize"
    );
    backend.inner_mut().set_memory_budget(Some(2 * MIB));
    let large = target_desc(1024, 1024, TextureFormat::Rgba8Unorm);
    let error = backend.create_texture(&large).unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::OutOfDeviceMemory);
    let events = backend.inner_mut().drain_events();
    let [MetalEvent::MemoryPressure {
        requested_bytes,
        resident_bytes: 0,
        budget_bytes,
    }] = events.as_slice()
    else {
        panic!("memory pressure event 하나: {events:?}");
    };
    assert!(*requested_bytes >= 4 * MIB && *budget_bytes == 2 * MIB);
    // 예산 안 할당은 성공하고 사용량에 잡히며, 파괴하면 돌아옵니다.
    let small = backend
        .create_texture(&target_desc(256, 256, TextureFormat::Rgba8Unorm))
        .expect("small");
    let used = backend.inner().memory_usage().resident_bytes;
    assert!((256 * 256 * 4..=2 * MIB).contains(&used), "{used}");
    let buffer_error = backend
        .create_buffer(&BufferDesc {
            size: 3 * MIB,
            usage: BufferUsage::Vertex,
        })
        .unwrap_err();
    assert_eq!(buffer_error.kind(), BackendErrorKind::OutOfDeviceMemory);
    backend.destroy_texture(small).expect("destroy");
    assert_eq!(backend.inner().memory_usage().resident_bytes, 0);
    // 예산 안이어도 device buffer 상한을 넘으면 Metal 할당 전에 거부하고 event를 냅니다.
    backend.inner_mut().set_memory_budget(Some(u64::MAX));
    backend.inner_mut().drain_events();
    let huge = backend
        .create_buffer(&BufferDesc {
            size: u64::MAX / 2,
            usage: BufferUsage::Vertex,
        })
        .unwrap_err();
    assert_eq!(huge.kind(), BackendErrorKind::OutOfDeviceMemory);
    assert!(matches!(
        backend.inner_mut().drain_events().as_slice(),
        [MetalEvent::MemoryPressure { .. }]
    ));
    assert_eq!(backend.validator().state(), CallOrderState::Idle);
}

#[test]
fn transient_pressure_fails_the_frame_and_purge_reclaims_memory() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(512, 512);
    let graph = transient_graph(512);
    // target(1 MiB)만 들어가는 예산: transient(1 MiB)는 거부됩니다.
    let target_bytes = env.backend.inner().memory_usage().resident_bytes;
    env.backend
        .inner_mut()
        .set_memory_budget(Some(target_bytes + MIB / 2));
    let error = env.try_render(&graph, Vec::new()).unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::OutOfDeviceMemory);
    assert_eq!(env.backend.validator().state(), CallOrderState::Idle);
    assert!(matches!(
        env.backend.inner_mut().drain_events().as_slice(),
        [MetalEvent::MemoryPressure { .. }]
    ));
    // 예산을 늘리면 같은 graph가 그려집니다.
    env.backend
        .inner_mut()
        .set_memory_budget(Some(target_bytes + 3 * MIB));
    env.render(&graph, Vec::new());
    assert_eq!(env.backend.inner().transient_texture_count(), 1);
    // transient가 차지한 공간이 있어야만 들어가는 할당은 transient pool을 비워 회수한 뒤 성공합니다.
    let reclaim = env
        .backend
        .create_texture(&target_desc(800, 800, TextureFormat::Rgba8Unorm))
        .expect("transient 회수 후 할당");
    assert_eq!(env.backend.inner().transient_texture_count(), 0);
    assert!(env.backend.inner_mut().drain_events().is_empty());
    env.backend.destroy_texture(reclaim).expect("destroy");
    // 다음 frame은 transient를 다시 만듭니다.
    env.render(&graph, Vec::new());
    assert_eq!(env.backend.inner().transient_texture_count(), 1);
}

#[test]
fn drawable_timeout_emits_events_and_skips_frames() {
    // 검증 기준(XI.8.7): drawable timeout에서 crash 없이 error event.
    if !metal_available() {
        return;
    }
    let mut backend = CheckedBackend::new(
        MetalBackend::with_detached_layer(LayerOptions::DEFAULT).expect("layer"),
    );
    backend
        .init(&device_config(8, 8, TextureFormat::Bgra8Unorm))
        .expect("init");
    let graph = surface_clear_graph(TextureFormat::Bgra8Unorm, unit_clear([true; 4]));
    let imports = bind_surface(&graph);
    backend.inner_mut().drawable_failures = 2;
    for frame in 1..=2 {
        assert_eq!(
            backend.encode(&request(frame, &graph, &imports)).unwrap(),
            EncodeOutcome::Skipped(SkipReason::NoDrawable)
        );
    }
    let events = backend.inner_mut().drain_events();
    assert_eq!(events.len(), 2);
    for (event, frame) in events.iter().zip(1..) {
        assert!(matches!(
            event,
            MetalEvent::DrawableTimeout { frame: id, .. } if id.0 == frame
        ));
        assert_eq!(event.kind(), BackendErrorKind::Timeout);
    }
    let EncodeOutcome::Encoded(encoded) = backend.encode(&request(3, &graph, &imports)).unwrap()
    else {
        panic!("drawable이 돌아오면 그립니다");
    };
    backend.schedule_present(&encoded).expect("present");
    backend.submit(encoded).expect("submit");
    backend.inner_mut().wait_idle().expect("wait");
    // 보관 event 수는 상한을 넘지 않습니다.
    backend.inner_mut().drawable_failures = (MAX_EVENTS + 10) as u32;
    for frame in 0..(MAX_EVENTS as u64 + 10) {
        backend
            .encode(&request(4 + frame, &graph, &imports))
            .expect("skip");
    }
    let events = backend.inner_mut().drain_events();
    assert_eq!(events.len(), MAX_EVENTS);
    assert!(matches!(
        events.last(),
        Some(MetalEvent::DrawableTimeout { frame, .. }) if frame.0 == 4 + MAX_EVENTS as u64 + 9
    ));
}

#[test]
fn missing_device_and_shader_failures_are_events() {
    if !metal_available() {
        return;
    }
    let mut backend = MetalBackend::headless();
    backend.fail_device_creation = true;
    let error = backend
        .init(&device_config(4, 4, TextureFormat::Bgra8Unorm))
        .unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::Unsupported);
    assert_eq!(
        backend.drain_events(),
        vec![MetalEvent::DeviceError {
            kind: BackendErrorKind::Unsupported,
            submission: None,
            command_buffer_error: None,
        }]
    );
    // device 없이 다른 호출은 구조화 오류이고, 다음 init은 성공합니다.
    assert_eq!(
        backend.poll().unwrap_err().kind(),
        BackendErrorKind::InvalidUsage
    );
    backend
        .init(&device_config(4, 4, TextureFormat::Bgra8Unorm))
        .expect("init");

    let mut env = Offscreen::new(4, 4);
    env.backend.inner_mut().use_shader_source("not metal");
    let vertices = env.buffer(BufferUsage::Vertex, &quad(0.0, 0.0, 4.0, 4.0, [1.0; 4]));
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let graph = single_pass(4, LoadOp::Clear, unit_clear([false; 4]));
    let error = env
        .try_render(
            &graph,
            vec![
                DrawCommand::SetPipeline(solid_key(TextureFormat::Rgba8Unorm, BlendState::Replace)),
                DrawCommand::SetVertexBuffer {
                    buffer: vertices,
                    offset: 0,
                },
                DrawCommand::SetUniforms {
                    buffer: uniforms,
                    offset: 0,
                    size: 32,
                },
            ],
        )
        .unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::PipelineCreation);
    assert_eq!(
        env.backend.inner_mut().drain_events(),
        vec![MetalEvent::PipelineFailure {
            stage: ShaderErrorStage::Compile
        }]
    );
}

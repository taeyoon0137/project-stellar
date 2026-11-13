//! `RendererBackend` 호출 순서 테스트 (XI.5.1).

use super::mock::MockBackend;
use super::*;
use crate::gpu_handle::{HandleTable, TextureKind};
use crate::placeholder::PLACEHOLDER_SAMPLER;
use crate::render_graph::{
    ClearColor, CompiledRenderGraph, ImportKind, LoadOp, PassKind, RenderGraphBuilder, StoreOp,
    TextureDesc, TextureFormat,
};

pub(crate) fn device_config(width: u32, height: u32) -> DeviceConfig {
    DeviceConfig {
        surface: SurfaceConfig {
            size: SurfaceSize { width, height },
            format: TextureFormat::Bgra8Unorm,
            color_space: ColorSpace::Srgb,
        },
        frames_in_flight: 3,
        timestamps: true,
    }
}

/// surface를 clear하고 그리는 pass 하나짜리 graph.
pub(crate) fn surface_graph() -> CompiledRenderGraph {
    let mut builder = RenderGraphBuilder::new();
    let surface = builder.import(
        "surface",
        TextureDesc::surface(TextureFormat::Bgra8Unorm),
        ImportKind::Surface,
    );
    let pass = builder.pass("main", PassKind::Raster);
    let out = builder
        .color(
            pass,
            surface,
            LoadOp::Clear,
            ClearColor::TRANSPARENT,
            StoreOp::Store,
        )
        .unwrap();
    builder.set_output(out);
    builder.finish().compile().unwrap()
}

/// external texture에 그리는 offscreen graph(present 없음).
fn offscreen_graph() -> CompiledRenderGraph {
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import(
        "export",
        TextureDesc::surface(TextureFormat::Rgba8Unorm),
        ImportKind::External,
    );
    let pass = builder.pass("export", PassKind::Raster);
    let out = builder
        .color(
            pass,
            target,
            LoadOp::Clear,
            ClearColor::TRANSPARENT,
            StoreOp::Store,
        )
        .unwrap();
    builder.set_output(out);
    builder.finish().compile().unwrap()
}

const SURFACE_IMPORT: [ImportBinding; 1] = [ImportBinding {
    resource: ResourceId(0),
    target: ImportTarget::Surface,
}];

fn texture_desc() -> GpuTextureDesc {
    GpuTextureDesc {
        width: 16,
        height: 16,
        format: TextureFormat::Rgba8Unorm,
        mip_levels: 1,
        samples: SampleCount::X1,
        usage: TextureUsage {
            sampled: true,
            render_target: false,
        },
    }
}

/// 다른 table이 발급한 texture handle(테스트 backend는 해석할 수 없음).
fn foreign_texture() -> TextureHandle {
    let mut table: HandleTable<TextureKind, ()> = HandleTable::new(99);
    table.insert(()).unwrap()
}

fn violation(result: Result<impl std::fmt::Debug, BackendError>) -> CallOrderError {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::InvalidUsage);
    error.call_order_violation().unwrap()
}

#[test]
fn trait_is_object_safe() {
    let mut backend: Box<dyn RendererBackend> = Box::new(CheckedBackend::new(MockBackend::new()));
    assert_eq!(backend.kind(), BackendKind::Mock);
    assert!(backend.init(&device_config(8, 8)).is_ok());
}

#[test]
fn full_frame_reaches_backend_in_contract_order() {
    // 검증 기준(XI.5.1): mock backend compile and call-order test.
    let graph = surface_graph();
    let mut backend = CheckedBackend::new(MockBackend::new());
    assert_eq!(backend.validator().state(), CallOrderState::Uninitialized);

    let caps = backend.init(&device_config(64, 32)).unwrap();
    assert_eq!(caps.backend, BackendKind::Mock);
    assert_eq!(backend.validator().state(), CallOrderState::Idle);

    let texture = backend.create_texture(&texture_desc()).unwrap();
    let region = TextureRegion {
        mip_level: 0,
        x: 0,
        y: 0,
        width: 16,
        height: 16,
    };
    backend
        .update_texture(texture, region, &[0u8; 16 * 16 * 4])
        .unwrap();
    let vertices = backend
        .create_buffer(&BufferDesc {
            size: 256,
            usage: BufferUsage::Vertex,
        })
        .unwrap();
    backend.update_buffer(vertices, 0, &[0u8; 64]).unwrap();

    let commands = [PassCommands {
        pass: graph.order()[0],
        commands: vec![
            DrawCommand::SetVertexBuffer {
                buffer: vertices,
                offset: 0,
            },
            DrawCommand::BindTexture {
                slot: 0,
                texture: TextureSource::Handle(texture),
                sampler: PLACEHOLDER_SAMPLER,
            },
            DrawCommand::Draw {
                vertex_start: 0,
                vertex_count: 6,
                instance_count: 1,
            },
        ],
    }];
    let request = FrameEncodeRequest {
        frame: FrameId(1),
        graph: &graph,
        passes: &commands,
        imports: &SURFACE_IMPORT,
    };
    assert!(request.targets_surface());
    assert_eq!(request.commands_for(graph.order()[0]), Some(&commands[0]));
    let EncodeOutcome::Encoded(frame) = backend.encode(&request).unwrap() else {
        panic!("surface frame은 encode되어야 합니다");
    };
    assert_eq!(frame.frame(), FrameId(1));
    assert_eq!(backend.validator().state(), CallOrderState::FrameOpen);
    backend.schedule_present(&frame).unwrap();
    assert!(backend.validator().present_scheduled());
    let submission = backend.submit(frame).unwrap();
    assert_eq!(backend.validator().state(), CallOrderState::Idle);
    assert_eq!(backend.validator().last_submission(), Some(submission));

    assert_eq!(
        backend.query_timestamps(submission).unwrap(),
        TimestampQuery::Pending
    );
    assert_eq!(backend.inner().in_flight(), 1);
    backend.inner_mut().complete_all();
    assert_eq!(backend.inner().in_flight(), 0);
    assert_eq!(backend.inner().completed().len(), 1);
    assert!(backend.inner().completed()[0].present);
    assert_eq!(backend.inner().completed()[0].frame, FrameId(1));
    assert_eq!(backend.inner().completed()[0].passes, graph.order());
    assert_eq!(
        backend.poll().unwrap(),
        GpuProgress {
            completed: Some(submission),
            presented: Some(submission)
        }
    );
    let TimestampQuery::Ready(timings) = backend.query_timestamps(submission).unwrap() else {
        panic!("완료된 submission은 timestamp가 있어야 합니다");
    };
    assert_eq!(timings.passes.len(), 1);
    backend.destroy_buffer(vertices).unwrap();
    backend.destroy_texture(texture).unwrap();

    assert_eq!(
        backend.inner().calls,
        vec![
            BackendCall::Init,
            BackendCall::CreateTexture,
            BackendCall::UpdateTexture,
            BackendCall::CreateBuffer,
            BackendCall::UpdateBuffer,
            BackendCall::Encode,
            BackendCall::SchedulePresent,
            BackendCall::Submit,
            BackendCall::QueryTimestamps,
            BackendCall::Poll,
            BackendCall::QueryTimestamps,
            BackendCall::DestroyBuffer,
            BackendCall::DestroyTexture,
        ]
    );
    assert_eq!(backend.inner().live_resources(), (0, 0));
}

#[test]
fn out_of_order_calls_never_reach_the_backend() {
    let graph = surface_graph();
    let mut backend = CheckedBackend::new(MockBackend::new());

    // init 전.
    assert_eq!(
        violation(backend.create_buffer(&BufferDesc {
            size: 4,
            usage: BufferUsage::Uniform
        })),
        CallOrderError::NotInitialized {
            call: BackendCall::CreateBuffer
        }
    );
    assert_eq!(
        violation(backend.poll()),
        CallOrderError::NotInitialized {
            call: BackendCall::Poll
        }
    );
    // frames in flight 범위.
    let mut bad = device_config(8, 8);
    bad.frames_in_flight = 4;
    assert_eq!(
        violation(backend.init(&bad)),
        CallOrderError::InvalidFramesInFlight { requested: 4 }
    );
    bad.frames_in_flight = 0;
    assert_eq!(
        violation(backend.init(&bad)),
        CallOrderError::InvalidFramesInFlight { requested: 0 }
    );
    assert!(backend.inner().calls.is_empty());

    backend.init(&device_config(8, 8)).unwrap();
    assert_eq!(
        violation(backend.init(&device_config(8, 8))),
        CallOrderError::AlreadyInitialized
    );
    // 열린 frame 없이 submit·present.
    let stray = EncodedFrame::new(FrameId(9), 9);
    assert_eq!(
        violation(backend.schedule_present(&stray)),
        CallOrderError::NoOpenFrame {
            call: BackendCall::SchedulePresent
        }
    );
    assert_eq!(
        violation(backend.submit(EncodedFrame::new(FrameId(9), 9))),
        CallOrderError::NoOpenFrame {
            call: BackendCall::Submit
        }
    );
    assert_eq!(
        violation(backend.query_timestamps(SubmissionId(1))),
        CallOrderError::UnknownSubmission {
            submission: SubmissionId(1)
        }
    );

    let request = FrameEncodeRequest {
        frame: FrameId(5),
        graph: &graph,
        passes: &[],
        imports: &SURFACE_IMPORT,
    };
    let EncodeOutcome::Encoded(frame) = backend.encode(&request).unwrap() else {
        panic!("encode되어야 합니다");
    };
    // 열린 frame 동안 금지되는 호출.
    assert_eq!(
        violation(backend.resize(SurfaceSize {
            width: 4,
            height: 4
        })),
        CallOrderError::FrameOpen {
            call: BackendCall::Resize
        }
    );
    assert_eq!(
        violation(backend.create_texture(&texture_desc())),
        CallOrderError::FrameOpen {
            call: BackendCall::CreateTexture
        }
    );
    let second = FrameEncodeRequest {
        frame: FrameId(6),
        ..request
    };
    assert_eq!(
        violation(backend.encode(&second)),
        CallOrderError::FrameOpen {
            call: BackendCall::Encode
        }
    );
    // 열린 frame과 다른 frame.
    assert_eq!(
        violation(backend.schedule_present(&stray)),
        CallOrderError::FrameMismatch {
            open: FrameId(5),
            given: FrameId(9)
        }
    );
    backend.schedule_present(&frame).unwrap();
    assert_eq!(
        violation(backend.schedule_present(&frame)),
        CallOrderError::PresentAlreadyScheduled
    );
    // poll은 열린 frame 중에도 허용됩니다.
    backend.poll().unwrap();
    backend.submit(frame).unwrap();

    // frame 번호는 엄격히 증가해야 합니다.
    assert_eq!(
        violation(backend.encode(&request)),
        CallOrderError::FrameNotIncreasing {
            previous: FrameId(5),
            frame: FrameId(5)
        }
    );

    // offscreen frame에는 present를 예약할 수 없습니다.
    let offscreen = offscreen_graph();
    let texture = backend.create_texture(&texture_desc()).unwrap();
    let export_import = [ImportBinding {
        resource: ResourceId(0),
        target: ImportTarget::Texture(texture),
    }];
    let export = FrameEncodeRequest {
        frame: FrameId(7),
        graph: &offscreen,
        passes: &[],
        imports: &export_import,
    };
    assert!(!export.targets_surface());
    let EncodeOutcome::Encoded(frame) = backend.encode(&export).unwrap() else {
        panic!("offscreen frame은 encode되어야 합니다");
    };
    assert_eq!(
        violation(backend.schedule_present(&frame)),
        CallOrderError::PresentWithoutSurface
    );
    backend.submit(frame).unwrap();

    assert_eq!(
        backend.inner().calls,
        vec![
            BackendCall::Init,
            BackendCall::Encode,
            BackendCall::SchedulePresent,
            BackendCall::Poll,
            BackendCall::Submit,
            BackendCall::CreateTexture,
            BackendCall::Encode,
            BackendCall::Submit,
        ]
    );
}

#[test]
fn encode_requests_are_validated() {
    let graph = surface_graph();
    let mut backend = CheckedBackend::new(MockBackend::new());
    backend.init(&device_config(8, 8)).unwrap();
    let pass = graph.order()[0];
    let base = FrameEncodeRequest {
        frame: FrameId(1),
        graph: &graph,
        passes: &[],
        imports: &SURFACE_IMPORT,
    };

    let unknown = [PassCommands::new(PassId(3))];
    assert_eq!(
        violation(backend.encode(&FrameEncodeRequest {
            passes: &unknown,
            ..base
        })),
        CallOrderError::UnknownPass { pass: PassId(3) }
    );
    let duplicate = [PassCommands::new(pass), PassCommands::new(pass)];
    assert_eq!(
        violation(backend.encode(&FrameEncodeRequest {
            passes: &duplicate,
            ..base
        })),
        CallOrderError::DuplicatePassCommands { pass }
    );
    assert_eq!(
        violation(backend.encode(&FrameEncodeRequest {
            imports: &[],
            ..base
        })),
        CallOrderError::MissingImport {
            resource: ResourceId(0)
        }
    );
    let wrong_kind = [ImportBinding {
        resource: ResourceId(0),
        target: ImportTarget::Texture(foreign_texture()),
    }];
    assert_eq!(
        violation(backend.encode(&FrameEncodeRequest {
            imports: &wrong_kind,
            ..base
        })),
        CallOrderError::InvalidImport {
            resource: ResourceId(0)
        }
    );
    let twice = [SURFACE_IMPORT[0], SURFACE_IMPORT[0]];
    assert_eq!(
        violation(backend.encode(&FrameEncodeRequest {
            imports: &twice,
            ..base
        })),
        CallOrderError::InvalidImport {
            resource: ResourceId(0)
        }
    );
    let not_imported = [ImportBinding {
        resource: ResourceId(5),
        target: ImportTarget::Surface,
    }];
    assert_eq!(
        violation(backend.encode(&FrameEncodeRequest {
            imports: &not_imported,
            ..base
        })),
        CallOrderError::InvalidImport {
            resource: ResourceId(5)
        }
    );
    // 거부된 요청은 backend에 닿지 않고 frame 번호도 소비하지 않습니다.
    assert_eq!(backend.inner().calls, vec![BackendCall::Init]);
    assert!(matches!(
        backend.encode(&base).unwrap(),
        EncodeOutcome::Encoded(_)
    ));
}

#[test]
fn skipped_frames_do_not_open_a_frame() {
    let graph = surface_graph();
    let mut backend = CheckedBackend::new(MockBackend::new());
    backend.init(&device_config(0, 0)).unwrap();
    let request = FrameEncodeRequest {
        frame: FrameId(1),
        graph: &graph,
        passes: &[],
        imports: &SURFACE_IMPORT,
    };
    assert_eq!(
        backend.encode(&request).unwrap(),
        EncodeOutcome::Skipped(SkipReason::EmptySurface)
    );
    assert_eq!(backend.validator().state(), CallOrderState::Idle);
    // skip한 frame 번호는 소비됩니다.
    assert_eq!(
        violation(backend.encode(&request)),
        CallOrderError::FrameNotIncreasing {
            previous: FrameId(1),
            frame: FrameId(1)
        }
    );
    backend
        .resize(SurfaceSize {
            width: 4,
            height: 4,
        })
        .unwrap();
    backend.inner_mut().no_drawable = true;
    assert_eq!(
        backend
            .encode(&FrameEncodeRequest {
                frame: FrameId(2),
                ..request
            })
            .unwrap(),
        EncodeOutcome::Skipped(SkipReason::NoDrawable)
    );
    backend.inner_mut().no_drawable = false;
    assert!(matches!(
        backend
            .encode(&FrameEncodeRequest {
                frame: FrameId(3),
                ..request
            })
            .unwrap(),
        EncodeOutcome::Encoded(_)
    ));
}

#[test]
fn device_lost_allows_only_reinit() {
    let graph = surface_graph();
    let mut backend = CheckedBackend::new(MockBackend::new());
    backend.init(&device_config(8, 8)).unwrap();
    let request = FrameEncodeRequest {
        frame: FrameId(1),
        graph: &graph,
        passes: &[],
        imports: &SURFACE_IMPORT,
    };
    let EncodeOutcome::Encoded(frame) = backend.encode(&request).unwrap() else {
        panic!("encode되어야 합니다");
    };
    backend.inner_mut().fail_next = Some((BackendCall::Submit, BackendErrorKind::DeviceLost));
    assert_eq!(
        backend.submit(frame).unwrap_err().kind(),
        BackendErrorKind::DeviceLost
    );
    assert_eq!(backend.validator().state(), CallOrderState::Lost);
    assert_eq!(
        violation(backend.poll()),
        CallOrderError::DeviceLost {
            call: BackendCall::Poll
        }
    );
    assert_eq!(
        violation(backend.create_texture(&texture_desc())),
        CallOrderError::DeviceLost {
            call: BackendCall::CreateTexture
        }
    );
    backend.init(&device_config(8, 8)).unwrap();
    assert_eq!(backend.validator().state(), CallOrderState::Idle);
    // 새 device는 submission 번호를 처음부터 셉니다. frame 번호는 계속 증가합니다.
    let EncodeOutcome::Encoded(frame) = backend
        .encode(&FrameEncodeRequest {
            frame: FrameId(2),
            ..request
        })
        .unwrap()
    else {
        panic!("encode되어야 합니다");
    };
    assert_eq!(backend.submit(frame).unwrap(), SubmissionId(1));
    // 다른 오류는 상태를 바꾸지 않습니다.
    backend.inner_mut().fail_next = Some((BackendCall::Poll, BackendErrorKind::Timeout));
    assert_eq!(
        backend.poll().unwrap_err().kind(),
        BackendErrorKind::Timeout
    );
    assert_eq!(backend.validator().state(), CallOrderState::Idle);
}

#[test]
fn backend_values_are_plain_values() {
    assert!(SurfaceSize {
        width: 0,
        height: 10
    }
    .is_empty());
    let error = BackendError::new(BackendErrorKind::Timeout, "drawable 대기");
    assert_eq!(error.detail(), "drawable 대기");
    assert!(error.to_string().contains("Timeout"));
    let order = BackendError::call_order(CallOrderError::AlreadyInitialized);
    assert!(order.to_string().contains("init"));
}

#[test]
fn stale_handles_fail_through_the_backend() {
    // 검증 기준(XI.5.2): stale handle fixture 실패 — backend trait을 통해서도 실패합니다.
    let mut backend = CheckedBackend::new(MockBackend::new());
    backend.init(&device_config(8, 8)).unwrap();
    let region = TextureRegion {
        mip_level: 0,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    let old = backend.create_texture(&texture_desc()).unwrap();
    backend.destroy_texture(old).unwrap();
    let new = backend.create_texture(&texture_desc()).unwrap();
    // 같은 slot을 재사용하지만 generation이 다릅니다.
    assert_eq!(old.index(), new.index());
    assert_ne!(old, new);
    let stale = backend.update_texture(old, region, &[0; 4]).unwrap_err();
    assert_eq!(stale.kind(), BackendErrorKind::InvalidUsage);
    assert_eq!(stale.detail(), "파괴된 GPU handle");
    assert_eq!(
        backend.destroy_texture(old).unwrap_err().detail(),
        "파괴된 GPU handle"
    );
    backend.update_texture(new, region, &[0; 4]).unwrap();
    assert_eq!(
        backend
            .inner()
            .native_texture(new)
            .map(|native| native.desc),
        Some(texture_desc())
    );
    assert!(backend.inner().native_texture(old).is_none());

    // 다른 table(다른 device)이 발급한 handle도 해석하지 않습니다.
    assert_eq!(
        backend
            .update_texture(foreign_texture(), region, &[0; 4])
            .unwrap_err()
            .detail(),
        "이전 device 세대의 GPU handle"
    );

    // device를 다시 만들면 이전 device의 handle은 모두 무효입니다.
    let buffer = backend
        .create_buffer(&BufferDesc {
            size: 16,
            usage: BufferUsage::Uniform,
        })
        .unwrap();
    backend.inner_mut().fail_next = Some((BackendCall::Poll, BackendErrorKind::DeviceLost));
    assert!(backend.poll().is_err());
    backend.init(&device_config(8, 8)).unwrap();
    assert_eq!(backend.inner().live_resources(), (0, 0));
    assert_eq!(
        backend
            .update_buffer(buffer, 0, &[0; 4])
            .unwrap_err()
            .detail(),
        "이전 device 세대의 GPU handle"
    );
    assert_eq!(
        backend
            .update_texture(new, region, &[0; 4])
            .unwrap_err()
            .detail(),
        "이전 device 세대의 GPU handle"
    );
    let fresh = backend
        .create_buffer(&BufferDesc {
            size: 16,
            usage: BufferUsage::Uniform,
        })
        .unwrap();
    assert_ne!(fresh, buffer);
    backend.update_buffer(fresh, 0, &[0; 16]).unwrap();
    // buffer 범위 밖 쓰기는 거부합니다.
    assert_eq!(
        backend
            .update_buffer(fresh, 8, &[0; 16])
            .unwrap_err()
            .kind(),
        BackendErrorKind::InvalidUsage
    );
}

#[test]
fn discard_closes_the_open_frame_without_submitting() {
    let graph = surface_graph();
    let mut backend = CheckedBackend::new(MockBackend::new());
    backend.init(&device_config(8, 8)).unwrap();
    assert_eq!(
        violation(backend.discard(EncodedFrame::new(FrameId(1), 1))),
        CallOrderError::NoOpenFrame {
            call: BackendCall::Discard
        }
    );
    let request = FrameEncodeRequest {
        frame: FrameId(1),
        graph: &graph,
        passes: &[],
        imports: &SURFACE_IMPORT,
    };
    let EncodeOutcome::Encoded(frame) = backend.encode(&request).unwrap() else {
        panic!("encode되어야 합니다");
    };
    // present를 예약한 뒤에도 버릴 수 있습니다.
    backend.schedule_present(&frame).unwrap();
    assert_eq!(
        violation(backend.discard(EncodedFrame::new(FrameId(9), 9))),
        CallOrderError::FrameMismatch {
            open: FrameId(1),
            given: FrameId(9)
        }
    );
    backend.discard(frame).unwrap();
    assert_eq!(backend.validator().state(), CallOrderState::Idle);
    assert_eq!(backend.validator().last_submission(), None);
    assert_eq!(backend.inner().discarded, vec![FrameId(1)]);
    assert_eq!(backend.inner().in_flight(), 0);
    // 버린 뒤 다음 frame을 정상 제출합니다.
    let EncodeOutcome::Encoded(next) = backend
        .encode(&FrameEncodeRequest {
            frame: FrameId(2),
            ..request
        })
        .unwrap()
    else {
        panic!("encode되어야 합니다");
    };
    assert_eq!(backend.submit(next).unwrap(), SubmissionId(1));
}

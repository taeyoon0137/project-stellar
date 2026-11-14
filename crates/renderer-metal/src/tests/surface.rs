//! XI.8.2: resize와 drawable 획득, drawable 없음 = frame skip.
//!
//! 창 밖 CAMetalLayer는 drawable 수 제한이 걸리지 않아(probe: 연속 8개 획득) 가려진 창의 nil drawable을
//! 자연적으로 만들 수 없습니다. 그래서 nil drawable은 backend의 테스트 주입
//! (`MetalBackend::drawable_failures`)으로 재현하고, 그 뒤 처리 경로(skip·frame 미개방·다음 frame
//! 정상 렌더)는 실제 GPU로 확인합니다.

use stellar_renderer_core::backend::{
    BackendErrorKind, CallOrderState, CheckedBackend, EncodeOutcome, RendererBackend, SkipReason,
    SurfaceSize,
};
use stellar_renderer_core::render_graph::{ClearColor, TextureFormat};

use super::*;
use crate::objc::autoreleasepool;
use crate::sys::CGSize;
use crate::{LayerOptions, MetalBackend, SurfaceStats};

const READBACK_LAYER: LayerOptions = LayerOptions {
    framebuffer_only: false,
    ..LayerOptions::DEFAULT
};

/// 그린 frame의 drawable pixel(BGRA) 또는 skip 이유.
enum FrameResult {
    Drawn(Vec<u8>),
    Skipped(SkipReason),
}

fn layer_backend(width: u32, height: u32) -> CheckedBackend<MetalBackend> {
    let mut backend =
        CheckedBackend::new(MetalBackend::with_detached_layer(READBACK_LAYER).expect("layer"));
    backend
        .init(&device_config(width, height, TextureFormat::Bgra8Unorm))
        .expect("init");
    backend
}

/// surface를 `clear`로 지우고 present까지 끝낸 뒤 drawable pixel을 읽습니다.
fn clear_frame(
    backend: &mut CheckedBackend<MetalBackend>,
    frame: u64,
    clear: ClearColor,
) -> FrameResult {
    let graph = surface_clear_graph(TextureFormat::Bgra8Unorm, clear);
    let imports = bind_surface(&graph);
    let encoded = match backend
        .encode(&request(frame, &graph, &imports))
        .expect("encode")
    {
        EncodeOutcome::Encoded(encoded) => encoded,
        EncodeOutcome::Skipped(reason) => return FrameResult::Skipped(reason),
    };
    let (capture, length) = backend
        .inner_mut()
        .capture_open_drawable()
        .expect("capture");
    backend.schedule_present(&encoded).expect("present");
    let submission = backend.submit(encoded).expect("submit");
    let progress = backend.inner_mut().wait_idle().expect("wait");
    assert_eq!(progress.presented, Some(submission));
    FrameResult::Drawn(buffer_bytes(&capture, length))
}

fn layer_size(backend: &CheckedBackend<MetalBackend>) -> CGSize {
    autoreleasepool(|| {
        backend
            .inner()
            .layer_surface()
            .expect("layer")
            .drawable_size()
    })
}

#[test]
fn minimized_surface_skips_without_requesting_drawables() {
    // 검증 기준(XI.8.2): minimized window crash 없음.
    if !metal_available() {
        return;
    }
    let mut backend = layer_backend(16, 8);
    let blue = unit_clear([false, false, true, true]);
    backend
        .resize(SurfaceSize {
            width: 0,
            height: 0,
        })
        .expect("minimize");
    for frame in 1..=5 {
        assert!(matches!(
            clear_frame(&mut backend, frame, blue),
            FrameResult::Skipped(SkipReason::EmptySurface)
        ));
        assert_eq!(backend.validator().state(), CallOrderState::Idle);
    }
    // 0 크기는 layer에 넣지 않습니다(마지막 유효 크기 유지).
    assert_eq!(
        layer_size(&backend),
        CGSize {
            width: 16.0,
            height: 8.0
        }
    );
    // 복원하면 새 크기로 다시 그립니다.
    backend
        .resize(SurfaceSize {
            width: 32,
            height: 12,
        })
        .expect("restore");
    let FrameResult::Drawn(pixels) = clear_frame(&mut backend, 6, blue) else {
        panic!("복원 뒤에는 그려야 합니다");
    };
    assert_eq!(pixels.len(), 32 * 12 * 4);
    assert!(all_pixels(&pixels, [255, 0, 0, 255]), "BGRA blue");
    assert_eq!(
        backend.inner().surface_stats(),
        SurfaceStats {
            resizes: 2,
            skipped_empty: 5,
            skipped_no_drawable: 0,
            drawable_size_resyncs: 0,
        }
    );
}

#[test]
fn occluded_surface_without_drawable_skips_frames() {
    // 검증 기준(XI.8.2): drawable 없음은 frame skip, occluded window crash 없음.
    if !metal_available() {
        return;
    }
    let mut backend = layer_backend(8, 8);
    let red = unit_clear([true, false, false, true]);
    // 먼저 한 frame을 정상 제출해 GPU에 작업이 남은 상태에서 가려짐을 흉내 냅니다.
    let graph = surface_clear_graph(TextureFormat::Bgra8Unorm, red);
    let imports = bind_surface(&graph);
    let EncodeOutcome::Encoded(first) = backend.encode(&request(1, &graph, &imports)).unwrap()
    else {
        panic!("첫 frame");
    };
    backend.schedule_present(&first).expect("present");
    let in_flight = backend.submit(first).expect("submit");
    backend.inner_mut().drawable_failures = 3;
    for frame in 2..=4 {
        assert!(matches!(
            clear_frame(&mut backend, frame, red),
            FrameResult::Skipped(SkipReason::NoDrawable)
        ));
        // skip은 frame을 열지 않으므로 resize·resource 호출이 막히지 않습니다.
        assert_eq!(backend.validator().state(), CallOrderState::Idle);
    }
    assert_eq!(
        backend.inner_mut().wait_idle().expect("wait").completed,
        Some(in_flight)
    );
    let FrameResult::Drawn(pixels) = clear_frame(&mut backend, 5, red) else {
        panic!("drawable이 돌아오면 다시 그려야 합니다");
    };
    assert!(all_pixels(&pixels, [0, 0, 255, 255]), "BGRA red");
    assert_eq!(backend.inner().surface_stats().skipped_no_drawable, 3);
}

#[test]
fn resize_sequence_renders_at_each_size() {
    // 검증 기준(XI.8.2): 최소화·복원·확대·축소·1×1 resize 연속에서 crash 없이 크기대로 렌더.
    if !metal_available() {
        return;
    }
    let mut backend = layer_backend(4, 4);
    let white = unit_clear([true; 4]);
    let sizes = [
        (64, 32),
        (0, 0),
        (64, 32),
        (1, 1),
        (0, 17),
        (17, 0),
        (128, 96),
        (3, 5),
    ];
    for (frame, (width, height)) in (1..).zip(sizes) {
        backend
            .resize(SurfaceSize { width, height })
            .expect("resize");
        let result = clear_frame(&mut backend, frame, white);
        if width == 0 || height == 0 {
            assert!(matches!(
                result,
                FrameResult::Skipped(SkipReason::EmptySurface)
            ));
            continue;
        }
        let FrameResult::Drawn(pixels) = result else {
            panic!("{width}x{height}에서 그려야 합니다");
        };
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        assert!(all_pixels(&pixels, [255; 4]));
        assert_eq!(
            layer_size(&backend),
            CGSize {
                width: f64::from(width),
                height: f64::from(height)
            }
        );
    }
    // 같은 크기 resize는 layer를 건드리지 않고 횟수에도 넣지 않습니다.
    let before = backend.inner().surface_stats().resizes;
    backend
        .resize(SurfaceSize {
            width: 3,
            height: 5,
        })
        .expect("same size");
    assert_eq!(backend.inner().surface_stats().resizes, before);
    // 상한을 넘는 크기는 거부하고 이전 크기를 유지합니다.
    let error = backend
        .resize(SurfaceSize {
            width: 16_385,
            height: 1,
        })
        .unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::InvalidUsage);
    assert_eq!(
        backend.inner().surface_size(),
        SurfaceSize {
            width: 3,
            height: 5
        }
    );
}

#[test]
fn layer_size_changed_behind_backend_is_resynced() {
    if !metal_available() {
        return;
    }
    let mut backend = layer_backend(16, 8);
    // 셸이 backend 모르게 layer 크기를 바꾼 상황.
    autoreleasepool(|| {
        backend
            .inner()
            .layer_surface()
            .expect("layer")
            .set_drawable_size(SurfaceSize {
                width: 7,
                height: 7,
            });
    });
    let green = unit_clear([false, true, false, true]);
    assert!(matches!(
        clear_frame(&mut backend, 1, green),
        FrameResult::Skipped(SkipReason::NoDrawable)
    ));
    assert_eq!(backend.inner().surface_stats().drawable_size_resyncs, 1);
    assert_eq!(
        layer_size(&backend),
        CGSize {
            width: 16.0,
            height: 8.0
        }
    );
    let FrameResult::Drawn(pixels) = clear_frame(&mut backend, 2, green) else {
        panic!("다시 맞춘 뒤에는 그려야 합니다");
    };
    assert_eq!(pixels.len(), 16 * 8 * 4);
}

#[test]
fn discarded_frame_releases_its_drawable() {
    if !metal_available() {
        return;
    }
    let mut backend = layer_backend(8, 8);
    let graph = surface_clear_graph(TextureFormat::Bgra8Unorm, unit_clear([true; 4]));
    let imports = bind_surface(&graph);
    // drawable 수(3)보다 많이 encode → present 예약 → discard 해도 drawable이 고갈되지 않습니다.
    for frame in 1..=8 {
        let EncodeOutcome::Encoded(encoded) =
            backend.encode(&request(frame, &graph, &imports)).unwrap()
        else {
            panic!("frame {frame}");
        };
        backend.schedule_present(&encoded).expect("present");
        backend.discard(encoded).expect("discard");
    }
    let progress = backend.poll().expect("poll");
    assert_eq!(progress.completed, None, "버린 frame은 GPU에 가지 않습니다");
    assert!(matches!(
        clear_frame(&mut backend, 9, unit_clear([true; 4])),
        FrameResult::Drawn(_)
    ));
}

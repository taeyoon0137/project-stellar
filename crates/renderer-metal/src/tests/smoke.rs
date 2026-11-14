//! XI.8.1: device·queue·CAMetalLayer 초기화와 smoke clear frame.

use std::ptr::NonNull;

use stellar_renderer_core::backend::{
    BackendErrorKind, BackendKind, CheckedBackend, EncodeOutcome, RendererBackend, SubmissionId,
    TimestampQuery,
};
use stellar_renderer_core::render_graph::TextureFormat;

use super::*;
use crate::objc::{autoreleasepool, ns_string, NSUInteger};
use crate::sys::CGSize;
use crate::{LayerOptions, MetalBackend};

const READBACK_LAYER: LayerOptions = LayerOptions {
    framebuffer_only: false,
    ..LayerOptions::DEFAULT
};

#[test]
fn headless_init_reports_metal_capabilities() {
    if !metal_available() {
        return;
    }
    let mut backend = MetalBackend::headless();
    assert!(!backend.has_surface());
    let caps = backend
        .init(&device_config(16, 16, TextureFormat::Bgra8Unorm))
        .expect("init");
    assert_eq!(caps.backend, BackendKind::Metal);
    assert_eq!(backend.kind(), BackendKind::Metal);
    assert_eq!(caps.max_texture_dimension, 16_384);
    assert_eq!(caps.stencil_bits, 8);
    assert!(!caps.display_p3_surface, "headless에는 surface가 없습니다");
    assert!(caps.msaa_x4, "Apple Silicon GPU는 4x MSAA를 지원합니다");
    assert_eq!(backend.capabilities(), Some(caps));
}

#[test]
fn offscreen_smoke_frame_clears_external_texture() {
    // 검증 기준(XI.8.1): macOS smoke frame clear 통과 — offscreen render-to-texture pixel readback.
    if !metal_available() {
        return;
    }
    let mut backend = CheckedBackend::new(MetalBackend::headless());
    backend
        .init(&device_config(8, 4, TextureFormat::Rgba8Unorm))
        .expect("init");
    let target = backend
        .create_texture(&target_desc(8, 4, TextureFormat::Rgba8Unorm))
        .expect("texture");
    let graph = external_clear_graph(
        8,
        4,
        TextureFormat::Rgba8Unorm,
        unit_clear([true, false, false, true]),
    );
    let imports = bind_first_import(&graph, target);
    let EncodeOutcome::Encoded(frame) = backend.encode(&request(1, &graph, &imports)).unwrap()
    else {
        panic!("offscreen frame은 skip하지 않습니다");
    };
    let submission = backend.submit(frame).expect("submit");
    assert_eq!(submission, SubmissionId(1));
    let progress = backend.inner_mut().wait_idle().expect("wait");
    assert_eq!(progress.completed, Some(submission));
    assert_eq!(
        progress.presented, None,
        "offscreen frame은 present하지 않습니다"
    );
    let pixels = backend.inner_mut().read_texture(target).expect("readback");
    assert_eq!(pixels.len(), 8 * 4 * 4);
    assert!(all_pixels(&pixels, [255, 0, 0, 255]), "{pixels:?}");
    // timestamp는 XI.8.6 전까지 지원하지 않습니다.
    assert_eq!(
        backend.query_timestamps(submission).unwrap(),
        TimestampQuery::Unsupported
    );
    backend.destroy_texture(target).expect("destroy");
}

#[test]
fn layer_smoke_frame_clears_drawable_and_presents() {
    // 검증 기준(XI.8.1): CAMetalLayer drawable smoke clear + present 예약.
    if !metal_available() {
        return;
    }
    let mut backend =
        CheckedBackend::new(MetalBackend::with_detached_layer(READBACK_LAYER).expect("layer"));
    let caps = backend
        .init(&device_config(16, 8, TextureFormat::Bgra8Unorm))
        .expect("init");
    assert!(caps.display_p3_surface);
    autoreleasepool(|| {
        let layer = backend.inner().layer_surface().expect("layer").layer();
        // SAFETY: CAMetalLayer getter: pixelFormat () -> NSUInteger, device () -> id.
        let (format, device): (NSUInteger, crate::objc::RawId) = unsafe {
            (
                layer.send(sel!("pixelFormat"), ()),
                layer.send(sel!("device"), ()),
            )
        };
        assert_eq!(format, 80, "MTLPixelFormatBGRA8Unorm");
        assert!(!device.is_null());
    });
    assert_eq!(
        backend
            .inner()
            .layer_surface()
            .expect("layer")
            .drawable_size(),
        CGSize {
            width: 16.0,
            height: 8.0
        }
    );
    let graph = surface_clear_graph(
        TextureFormat::Bgra8Unorm,
        unit_clear([false, true, false, true]),
    );
    let imports = bind_surface(&graph);
    let EncodeOutcome::Encoded(frame) = backend.encode(&request(1, &graph, &imports)).unwrap()
    else {
        panic!("drawable이 있어야 합니다");
    };
    let (capture, length) = backend
        .inner_mut()
        .capture_open_drawable()
        .expect("capture");
    backend.schedule_present(&frame).expect("present");
    let submission = backend.submit(frame).expect("submit");
    let progress = backend.inner_mut().wait_idle().expect("wait");
    assert_eq!(progress.completed, Some(submission));
    assert_eq!(progress.presented, Some(submission));
    // BGRA byte 순서: green = (b 0, g 255, r 0, a 255).
    let pixels = buffer_bytes(&capture, length);
    assert_eq!(length, 16 * 8 * 4);
    assert!(all_pixels(&pixels, [0, 255, 0, 255]), "{pixels:?}");
}

#[test]
fn layer_rejects_unsupported_formats_and_foreign_objects() {
    if !metal_available() {
        return;
    }
    // CAMetalLayer는 RGBA8 drawable을 받지 않습니다(예외 대신 구조화 오류).
    let mut backend = MetalBackend::with_detached_layer(LayerOptions::DEFAULT).expect("layer");
    let error = backend
        .init(&device_config(4, 4, TextureFormat::Rgba8Unorm))
        .unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::Unsupported);
    let error = backend
        .init(&device_config(4, 4, TextureFormat::Depth32FloatStencil8))
        .unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::InvalidUsage);
    let error = backend
        .init(&device_config(16_385, 4, TextureFormat::Bgra8Unorm))
        .unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::InvalidUsage);
    // 같은 backend로 올바른 format init은 성공합니다.
    assert!(backend
        .init(&device_config(4, 4, TextureFormat::Rgba16Float))
        .is_ok());

    let bad_count = LayerOptions {
        maximum_drawable_count: 4,
        ..LayerOptions::DEFAULT
    };
    assert_eq!(
        MetalBackend::with_detached_layer(bad_count)
            .unwrap_err()
            .kind(),
        BackendErrorKind::InvalidUsage
    );
    autoreleasepool(|| {
        let not_a_layer = ns_string("CAMetalLayer가 아닌 객체").expect("NSString");
        let raw = NonNull::new(not_a_layer.as_raw()).expect("non-null");
        // SAFETY: raw는 살아 있는 NSString 객체입니다.
        let error = unsafe { MetalBackend::with_layer(raw, LayerOptions::DEFAULT) }.unwrap_err();
        assert_eq!(error.kind(), BackendErrorKind::InvalidUsage);
    });
}

#[test]
fn misuse_is_reported_as_structured_errors() {
    if !metal_available() {
        return;
    }
    let mut backend = MetalBackend::headless();
    // init 전 호출.
    assert_eq!(
        backend
            .create_texture(&target_desc(4, 4, TextureFormat::Rgba8Unorm))
            .unwrap_err()
            .kind(),
        BackendErrorKind::InvalidUsage
    );
    backend
        .init(&device_config(4, 4, TextureFormat::Bgra8Unorm))
        .expect("init");
    // surface 없는 backend에 surface graph.
    let graph = surface_clear_graph(TextureFormat::Bgra8Unorm, unit_clear([true; 4]));
    let imports = bind_surface(&graph);
    assert_eq!(
        backend
            .encode(&request(1, &graph, &imports))
            .unwrap_err()
            .kind(),
        BackendErrorKind::InvalidUsage
    );
    // external texture 크기·format 불일치.
    let target = backend
        .create_texture(&target_desc(4, 4, TextureFormat::Rgba8Unorm))
        .expect("texture");
    for graph in [
        external_clear_graph(8, 4, TextureFormat::Rgba8Unorm, unit_clear([true; 4])),
        external_clear_graph(4, 4, TextureFormat::Bgra8Unorm, unit_clear([true; 4])),
    ] {
        let imports = bind_first_import(&graph, target);
        assert_eq!(
            backend
                .encode(&request(1, &graph, &imports))
                .unwrap_err()
                .kind(),
            BackendErrorKind::InvalidUsage
        );
    }
    // 잘못된 descriptor는 Metal에 닿기 전에 거부합니다.
    assert_eq!(
        backend
            .create_texture(&target_desc(0, 4, TextureFormat::Rgba8Unorm))
            .unwrap_err()
            .kind(),
        BackendErrorKind::InvalidUsage
    );
    // 파괴한 texture handle은 stale입니다.
    backend.destroy_texture(target).expect("destroy");
    assert_eq!(
        backend.read_texture(target).unwrap_err().kind(),
        BackendErrorKind::InvalidUsage
    );
}

#[test]
fn reinit_invalidates_previous_device_handles() {
    if !metal_available() {
        return;
    }
    let mut backend = MetalBackend::headless();
    backend
        .init(&device_config(4, 4, TextureFormat::Bgra8Unorm))
        .expect("init");
    let old = backend
        .create_texture(&target_desc(4, 4, TextureFormat::Rgba8Unorm))
        .expect("texture");
    backend
        .init(&device_config(4, 4, TextureFormat::Bgra8Unorm))
        .expect("re-init");
    assert_eq!(
        backend.destroy_texture(old).unwrap_err().kind(),
        BackendErrorKind::InvalidUsage
    );
    // 새 device의 submission 번호는 1부터입니다.
    let target = backend
        .create_texture(&target_desc(4, 4, TextureFormat::Rgba8Unorm))
        .expect("texture");
    let graph = external_clear_graph(4, 4, TextureFormat::Rgba8Unorm, unit_clear([true; 4]));
    let imports = bind_first_import(&graph, target);
    let EncodeOutcome::Encoded(frame) = backend.encode(&request(1, &graph, &imports)).unwrap()
    else {
        panic!("encode");
    };
    assert_eq!(backend.submit(frame).unwrap(), SubmissionId(1));
    backend.wait_idle().expect("wait");
    assert!(all_pixels(
        &backend.read_texture(target).expect("readback"),
        [255; 4]
    ));
}

//! DEV-MAC 실제 창 검사. 기본 unit test/CI에 포함하지 않습니다.
//! cargo run -p stellar-renderer-metal --example window_surface
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "../src/objc.rs"]
mod objc;

#[cfg(target_os = "macos")]
mod check {
    use super::objc::{autoreleasepool, class, sel, send, Id, RawId};
    use std::{
        ptr::NonNull,
        sync::mpsc,
        time::{Duration, Instant},
    };
    use stellar_renderer_core::{
        backend::*,
        placeholder::{placeholder_texture, PlaceholderKind},
        render_graph::*,
    };
    use stellar_renderer_metal::{LayerOptions, MetalBackend};

    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}

    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct Size {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Debug, Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Size,
    }

    // SAFETY: 모든 AppKit 호출은 main thread에서 SDK selector의 ABI대로 호출합니다.
    unsafe fn window() -> Id {
        let raw: RawId = unsafe { send(class!("NSWindow").unwrap().as_raw(), sel!("alloc"), ()) };
        let raw = unsafe {
            send(
                raw,
                sel!("initWithContentRect:styleMask:backing:defer:"),
                (
                    Rect {
                        origin: Point { x: 120.0, y: 120.0 },
                        size: Size {
                            width: 480.0,
                            height: 320.0,
                        },
                    },
                    15_usize,
                    2_usize,
                    false,
                ),
            )
        };
        let window = unsafe { Id::from_owned(raw) }.unwrap();
        unsafe {
            window.send::<_, ()>(sel!("setReleasedWhenClosed:"), (false,));
        }
        window
    }

    unsafe fn pump(app: RawId) {
        unsafe {
            let date: RawId = send(
                class!("NSDate").unwrap().as_raw(),
                sel!("dateWithTimeIntervalSinceNow:"),
                (0.01_f64,),
            );
            let mode: RawId = send(
                class!("NSString").unwrap().as_raw(),
                sel!("stringWithUTF8String:"),
                (c"kCFRunLoopDefaultMode".as_ptr(),),
            );
            let event: RawId = send(
                app,
                sel!("nextEventMatchingMask:untilDate:inMode:dequeue:"),
                (usize::MAX, date, mode, true),
            );
            if !event.is_null() {
                send::<_, ()>(app, sel!("sendEvent:"), (event,));
            }
            send::<_, ()>(app, sel!("updateWindows"), ());
        }
    }

    unsafe fn until(app: RawId, predicate: impl Fn() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(4);
        while !predicate() && Instant::now() < end {
            autoreleasepool(|| unsafe { pump(app) });
        }
        predicate()
    }

    fn render(backend: &mut MetalBackend, id: u64, size: SurfaceSize) -> String {
        backend.resize(size).unwrap();
        let mut builder = RenderGraphBuilder::new();
        let target = builder.import(
            "surface",
            TextureDesc::surface(TextureFormat::Bgra8UnormSrgb),
            ImportKind::Surface,
        );
        let pass = builder.pass("clear", PassKind::Raster);
        let output = builder
            .color(
                pass,
                target,
                LoadOp::Clear,
                ClearColor::TRANSPARENT,
                StoreOp::Store,
            )
            .unwrap();
        builder.set_output(output);
        let graph = builder.finish().compile().unwrap();
        let imports = [ImportBinding {
            resource: output.resource,
            target: ImportTarget::Surface,
        }];
        let result = match backend
            .encode(&FrameEncodeRequest {
                frame: FrameId(id),
                graph: &graph,
                passes: &[],
                imports: &imports,
            })
            .unwrap()
        {
            EncodeOutcome::Skipped(reason) => format!("Skipped({reason:?})"),
            EncodeOutcome::Encoded(frame) => {
                backend.schedule_present(&frame).unwrap();
                let submission = backend.submit(frame).unwrap();
                let deadline = Instant::now() + Duration::from_secs(4);
                loop {
                    if backend.poll().unwrap().presented == Some(submission) {
                        break;
                    }
                    assert!(Instant::now() < deadline, "GPU completion timeout");
                    std::thread::yield_now();
                }
                format!("Presented({submission:?})")
            }
        };
        format!(
            "{result} size={}x{} stats={:?}",
            size.width,
            size.height,
            backend.surface_stats()
        )
    }

    extern "C" {
        static _dispatch_main_q: std::ffi::c_void;
        fn dispatch_async_f(
            queue: *const std::ffi::c_void,
            context: *mut std::ffi::c_void,
            work: extern "C" fn(*mut std::ffi::c_void),
        );
    }

    extern "C" fn launched(_: *mut std::ffi::c_void) {
        // unwind가 C callback 경계를 넘지 않게 하고 모든 scoped 소유자를 먼저 해제합니다.
        let result = std::panic::catch_unwind(check);
        std::process::exit(if matches!(result, Ok(true)) { 0 } else { 1 });
    }

    pub fn run() {
        // 실제 앱과 동일한 NSApplication.run 경로에서 launch event와 WindowServer 통지를 처리합니다.
        unsafe {
            let app: RawId = send(
                class!("NSApplication").unwrap().as_raw(),
                sel!("sharedApplication"),
                (),
            );
            send::<_, bool>(app, sel!("setActivationPolicy:"), (0_isize,));
            dispatch_async_f(
                std::ptr::addr_of!(_dispatch_main_q),
                std::ptr::null_mut(),
                launched,
            );
            send::<_, ()>(app, sel!("run"), ());
            panic!("검증 callback 완료 전에 NSApplication.run이 종료됐습니다");
        }
    }

    fn check() -> bool {
        autoreleasepool(|| unsafe {
            let app: RawId = send(
                class!("NSApplication").unwrap().as_raw(),
                sel!("sharedApplication"),
                (),
            );
            let window = window();
            let cover = self::window();
            let layer = Id::from_owned(send(
                class!("CAMetalLayer").unwrap().as_raw(),
                sel!("new"),
                (),
            ))
            .unwrap();
            let view: RawId = window.send(sel!("contentView"), ());
            send::<_, ()>(view, sel!("setWantsLayer:"), (true,));
            send::<_, ()>(view, sel!("setLayer:"), (layer.as_raw(),));
            window.send::<_, ()>(
                sel!("makeKeyAndOrderFront:"),
                (std::ptr::null_mut::<std::ffi::c_void>(),),
            );
            send::<_, ()>(app, sel!("activateIgnoringOtherApps:"), (true,));
            let visible = || window.send::<_, usize>(sel!("occlusionState"), ()) & 2 != 0;
            let shown = until(app, || {
                visible() && window.send::<_, bool>(sel!("isKeyWindow"), ())
            });
            println!(
                "shown={shown} visible={} key={} active={}",
                visible(),
                window.send::<_, bool>(sel!("isKeyWindow"), ()),
                send::<_, bool>(app, sel!("isActive"), ())
            );
            assert!(
                visible(),
                "실제 창 표시 실패: DEV-MAC 로그인 데스크톱에서 재실행 필요"
            );
            let scale: f64 = window.send(sel!("backingScaleFactor"), ());
            let size = SurfaceSize {
                width: (480.0 * scale) as u32,
                height: (320.0 * scale) as u32,
            };
            let address = layer.as_raw() as usize;
            let mut covered = false;
            // main의 layer 소유권은 scoped worker join 뒤까지 유지합니다. backend는 worker 안에서만 생성·사용·해제합니다.
            std::thread::scope(|scope| {
                let (tx, rx) = mpsc::channel::<SurfaceSize>();
                let (results, completed) = mpsc::channel();
                scope.spawn(move || {
                    let mut backend = MetalBackend::with_layer(
                        NonNull::new(address as *mut _).unwrap(),
                        LayerOptions::default(),
                    )
                    .unwrap();
                    backend
                        .init(&DeviceConfig {
                            surface: SurfaceConfig {
                                size,
                                format: TextureFormat::Bgra8UnormSrgb,
                                color_space: placeholder_texture(PlaceholderKind::Missing)
                                    .color_space(),
                            },
                            frames_in_flight: 3,
                            timestamps: false,
                        })
                        .unwrap();
                    for (index, size) in rx.iter().enumerate() {
                        results
                            .send(render(&mut backend, index as u64 + 1, size))
                            .unwrap();
                    }
                    // 각 제출의 GPU 완료를 위에서 확인했으므로 대기 없이 worker 소유 backend를 해제합니다.
                    backend.poll().unwrap();
                });
                let frame = |label: &str, requested| {
                    tx.send(requested).unwrap();
                    let end = Instant::now() + Duration::from_secs(8);
                    let result = loop {
                        match completed.try_recv() {
                            Ok(result) => break result,
                            Err(mpsc::TryRecvError::Empty) => {}
                            Err(mpsc::TryRecvError::Disconnected) => panic!("render worker 종료"),
                        }
                        assert!(Instant::now() < end, "frame timeout");
                        autoreleasepool(|| pump(app));
                    };
                    println!(
                        "{label}: mini={} visible={} {result}",
                        window.send::<_, bool>(sel!("isMiniaturized"), ()),
                        visible()
                    );
                    result
                };
                assert!(frame("initial", size).starts_with("Presented"));
                window.send::<_, ()>(
                    sel!("miniaturize:"),
                    (std::ptr::null_mut::<std::ffi::c_void>(),),
                );
                assert!(
                    until(app, || window.send(sel!("isMiniaturized"), ())),
                    "최소화 실패"
                );
                for _ in 0..3 {
                    assert!(frame(
                        "minimized",
                        SurfaceSize {
                            width: 0,
                            height: 0
                        }
                    )
                    .starts_with("Skipped(EmptySurface)"));
                }
                window.send::<_, ()>(
                    sel!("deminiaturize:"),
                    (std::ptr::null_mut::<std::ffi::c_void>(),),
                );
                assert!(until(app, || visible()
                    && !window.send::<_, bool>(sel!("isMiniaturized"), ())));
                assert!(frame("restored", size).starts_with("Presented"));
                // 더 큰 불투명 NSWindow가 대상 창을 완전히 덮습니다(orderOut 모방 아님).
                let mut cover_frame: Rect = window.send(sel!("frame"), ());
                cover_frame.origin.x -= 20.0;
                cover_frame.origin.y -= 20.0;
                cover_frame.size.width += 40.0;
                cover_frame.size.height += 40.0;
                cover.send::<_, ()>(sel!("setFrame:display:"), (cover_frame, true));
                cover.send::<_, ()>(sel!("setOpaque:"), (true,));
                let white: RawId =
                    send(class!("NSColor").unwrap().as_raw(), sel!("whiteColor"), ());
                cover.send::<_, ()>(sel!("setBackgroundColor:"), (white,));
                cover.send::<_, ()>(sel!("setLevel:"), (3_isize,));
                cover.send::<_, ()>(sel!("orderFrontRegardless"), ());
                cover.send::<_, ()>(sel!("displayIfNeeded"), ());
                covered = until(app, || !visible());
                println!("covered={covered} cover_visible={} target_frame={:?} cover_frame={cover_frame:?}",
                    cover.send::<_, bool>(sel!("isVisible"), ()), window.send::<_, Rect>(sel!("frame"), ()));
                if covered {
                    // 가림이 항상 nil drawable을 뜻하지 않습니다. 실제 획득 결과도 기록합니다.
                    frame("occluded-acquire", size);
                    for _ in 0..3 {
                        assert!(frame(
                            "occluded-suspended",
                            SurfaceSize {
                                width: 0,
                                height: 0
                            }
                        )
                        .starts_with("Skipped(EmptySurface)"));
                    }
                } else {
                    eprintln!("가림 상태 미관측: occluded 프레임 검증 미실행");
                }
                cover.send::<_, ()>(
                    sel!("orderOut:"),
                    (std::ptr::null_mut::<std::ffi::c_void>(),),
                );
                assert!(until(app, visible));
                assert!(frame("uncovered", size).starts_with("Presented"));
                window.send::<_, ()>(
                    sel!("setContentSize:"),
                    (Size {
                        width: 520.0,
                        height: 360.0,
                    },),
                );
                assert!(frame(
                    "resized",
                    SurfaceSize {
                        width: (520.0 * scale) as u32,
                        height: (360.0 * scale) as u32
                    }
                )
                .starts_with("Presented"));
                window.send::<_, ()>(
                    sel!("orderOut:"),
                    (std::ptr::null_mut::<std::ffi::c_void>(),),
                );
                assert!(until(app, || !visible()));
                assert!(frame(
                    "ordered-out",
                    SurfaceSize {
                        width: 0,
                        height: 0
                    }
                )
                .starts_with("Skipped(EmptySurface)"));
                window.send::<_, ()>(sel!("orderFrontRegardless"), ());
                assert!(until(app, visible));
                assert!(frame(
                    "ordered-in",
                    SurfaceSize {
                        width: (520.0 * scale) as u32,
                        height: (360.0 * scale) as u32
                    }
                )
                .starts_with("Presented"));
                drop(tx);
            });
            cover.send::<_, ()>(sel!("close"), ());
            window.send::<_, ()>(sel!("close"), ());
            println!("window_surface: minimize/restore/resize/order-out/in=passed worker=joined covered={covered} activated={shown}");
            covered && shown
        })
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    check::run();
    #[cfg(not(target_os = "macos"))]
    panic!("DEV-MAC 전용 검사");
}

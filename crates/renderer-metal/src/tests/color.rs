//! XI.10.1: 선형 working space 입력과 target format별 출력 encode — CPU 기준과 GPU pixel 비교.
//!
//! 단색 quad를 working 값(`stellar_renderer_core::color::to_working`)으로 그리고, readback byte를 같은 모듈의
//! CPU 기준(`encode_pixel`)과 비교합니다. 8-bit format은 채널당 1 LSB(GPU 반올림 방향 차이), half float는 half
//! 반올림 한 단계까지 허용합니다.

use stellar_renderer_core::backend::{
    BackendKind, BlendState, BufferUsage, CheckedBackend, DeviceCapabilities, DrawCommand,
    EncodeOutcome, PassCommands, RendererBackend,
};
use stellar_renderer_core::color::{
    decode_pixel, encode_pixel, output_encoding, to_working, ColorSpace, OutputEncoding,
    WorkingColor,
};
use stellar_renderer_core::output_color::{OutputColor, OutputColorPlan};
use stellar_renderer_core::render_graph::{
    AttachmentSize, CompiledRenderGraph, ImportKind, LoadOp, PassId, PassKind, RenderGraphBuilder,
    SampleCount, StoreOp, TextureDesc, TextureFormat,
};

use super::geometry::{draw_uniforms, quad, solid_key};
use super::*;
use crate::surface::color_space_name;
use crate::sys::{
    kCGColorSpaceDisplayP3, kCGColorSpaceExtendedLinearDisplayP3, kCGColorSpaceExtendedLinearSRGB,
    kCGColorSpaceLinearDisplayP3, kCGColorSpaceLinearSRGB, kCGColorSpaceSRGB,
};
use crate::MetalBackend;

const CELL: u32 = 4;

fn caps(display_p3: bool) -> DeviceCapabilities {
    DeviceCapabilities {
        backend: BackendKind::Metal,
        max_texture_dimension: 16_384,
        display_p3_surface: display_p3,
        timestamp_query: false,
        msaa_x4: true,
        stencil_bits: 8,
    }
}

fn color(space: ColorSpace, rgb: [f32; 3], alpha: f32) -> OutputColor {
    OutputColor { space, rgb, alpha }
}

/// 입력 색 표본: sRGB·P3, 불투명·반투명, 어두운 값(선형 구간)과 gamut 밖 P3.
fn samples() -> Vec<OutputColor> {
    vec![
        color(ColorSpace::Srgb, [1.0, 0.0, 0.0], 1.0),
        color(ColorSpace::Srgb, [0.5, 0.5, 0.5], 1.0),
        color(ColorSpace::Srgb, [0.02, 0.2, 0.9], 1.0),
        color(ColorSpace::Srgb, [0.8, 0.4, 0.1], 0.5),
        color(ColorSpace::Srgb, [0.3, 0.9, 0.6], 0.25),
        color(ColorSpace::DisplayP3, [1.0, 0.0, 0.0], 1.0),
        color(ColorSpace::DisplayP3, [0.2, 0.7, 0.3], 0.75),
        color(ColorSpace::DisplayP3, [0.6, 0.6, 0.6], 1.0),
    ]
}

/// `format` external target 하나를 투명으로 지우고 그리는 graph.
fn target_graph(
    width: u32,
    height: u32,
    format: TextureFormat,
    clear: WorkingColor,
) -> CompiledRenderGraph {
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import(
        "target",
        TextureDesc {
            format,
            size: AttachmentSize::Fixed { width, height },
            samples: SampleCount::X1,
        },
        ImportKind::External,
    );
    let pass = builder.pass("draw", PassKind::Raster);
    let written = builder
        .color(
            pass,
            target,
            LoadOp::Clear,
            clear.to_clear().expect("finite clear"),
            StoreOp::Store,
        )
        .expect("color");
    builder.set_output(written);
    builder.finish().compile().expect("compile")
}

/// `quads`(x0, y0, x1, y1, working 색, blend)를 순서대로 그리고 readback합니다.
fn render(
    format: TextureFormat,
    width: u32,
    height: u32,
    clear: WorkingColor,
    quads: &[([f32; 4], WorkingColor, BlendState)],
) -> Vec<u8> {
    let mut backend = CheckedBackend::new(MetalBackend::headless());
    backend
        .init(&device_config(width, height, TextureFormat::Bgra8Unorm))
        .expect("init");
    let target = backend
        .create_texture(&target_desc(width, height, format))
        .expect("target");
    let mut vertices = Vec::new();
    for (rect, color, _) in quads {
        vertices.extend(quad(rect[0], rect[1], rect[2], rect[3], color.rgba));
    }
    let make = |backend: &mut CheckedBackend<MetalBackend>, usage, data: &[u8]| {
        let handle = backend
            .create_buffer(&stellar_renderer_core::backend::BufferDesc {
                size: data.len() as u64,
                usage,
            })
            .expect("buffer");
        backend.update_buffer(handle, 0, data).expect("update");
        handle
    };
    let vertex_buffer = make(&mut backend, BufferUsage::Vertex, &vertices);
    let uniform_buffer = make(
        &mut backend,
        BufferUsage::Uniform,
        &draw_uniforms([1.0; 4], 0.5),
    );
    let mut commands = vec![
        DrawCommand::SetVertexBuffer {
            buffer: vertex_buffer,
            offset: 0,
        },
        DrawCommand::SetUniforms {
            buffer: uniform_buffer,
            offset: 0,
            size: 32,
        },
    ];
    for (index, (_, _, blend)) in quads.iter().enumerate() {
        commands.push(DrawCommand::SetPipeline(solid_key(format, *blend)));
        commands.push(DrawCommand::Draw {
            vertex_start: index as u32 * 6,
            vertex_count: 6,
            instance_count: 1,
        });
    }
    let graph = target_graph(width, height, format, clear);
    let imports = bind_first_import(&graph, target);
    let passes = [PassCommands {
        pass: PassId(0),
        commands,
    }];
    let request = stellar_renderer_core::backend::FrameEncodeRequest {
        frame: stellar_renderer_core::backend::FrameId(1),
        graph: &graph,
        passes: &passes,
        imports: &imports,
    };
    let EncodeOutcome::Encoded(frame) = backend.encode(&request).expect("encode") else {
        panic!("offscreen frame은 skip하지 않습니다");
    };
    backend.submit(frame).expect("submit");
    backend.inner_mut().wait_idle().expect("wait");
    backend.inner_mut().read_texture(target).expect("readback")
}

/// 8-bit는 byte 차이 1, half float는 값 차이 `half_eps`까지 같은 pixel입니다.
#[track_caller]
fn assert_pixel(
    format: TextureFormat,
    actual: &[u8],
    expected: &[u8],
    surface: ColorSpace,
    at: (u32, u32),
) {
    match output_encoding(format) {
        Some(OutputEncoding::LinearFloat) => {
            let a = decode_pixel(actual, format, surface).unwrap();
            let e = decode_pixel(expected, format, surface).unwrap();
            for (x, y) in a.rgba.iter().zip(e.rgba.iter()) {
                assert!(
                    (x - y).abs() <= 2e-3,
                    "{format:?} {at:?}: GPU {a:?} vs CPU {e:?}"
                );
            }
        }
        _ => {
            for (x, y) in actual.iter().zip(expected.iter()) {
                assert!(
                    x.abs_diff(*y) <= 1,
                    "{format:?} {at:?}: GPU {actual:?} vs CPU {expected:?}"
                );
            }
        }
    }
}

#[test]
fn solid_colors_match_cpu_reference_for_every_output_encoding() {
    // 검증 기준(XI.10.1): CPU reference와 GPU pixel 허용 오차 통과.
    if !metal_available() {
        return;
    }
    let colors = samples();
    let width = CELL * colors.len() as u32;
    for (surface, p3_caps) in [(ColorSpace::Srgb, false), (ColorSpace::DisplayP3, true)] {
        let plan = OutputColorPlan::negotiate(surface, &caps(p3_caps));
        assert_eq!(plan.surface(), surface);
        let working: Vec<WorkingColor> = colors.iter().map(|c| to_working(&plan, *c)).collect();
        for format in [
            TextureFormat::Rgba8UnormSrgb,
            TextureFormat::Bgra8UnormSrgb,
            TextureFormat::Rgba8Unorm,
            TextureFormat::Rgba16Float,
        ] {
            let quads: Vec<_> = working
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let x0 = (i as u32 * CELL) as f32;
                    (
                        [x0, 0.0, x0 + CELL as f32, CELL as f32],
                        *c,
                        BlendState::Replace,
                    )
                })
                .collect();
            let pixels = render(format, width, CELL, WorkingColor::TRANSPARENT, &quads);
            let bpp = stellar_renderer_core::color::bytes_per_pixel(format).unwrap();
            for y in 0..CELL {
                for x in 0..width {
                    let offset = ((y * width + x) as usize) * bpp;
                    let expected =
                        encode_pixel(working[(x / CELL) as usize], format, surface).unwrap();
                    assert_pixel(
                        format,
                        &pixels[offset..offset + bpp],
                        &expected,
                        surface,
                        (x, y),
                    );
                }
            }
        }
    }
}

#[test]
fn clear_values_and_linear_blending_match_cpu_reference() {
    // clear 값은 working 값이고, sRGB target의 blend는 decode한 선형 값으로 계산됩니다.
    if !metal_available() {
        return;
    }
    let plan = OutputColorPlan::negotiate(ColorSpace::Srgb, &caps(false));
    let background = to_working(&plan, color(ColorSpace::Srgb, [0.1, 0.3, 0.8], 1.0));
    let overlay = to_working(&plan, color(ColorSpace::Srgb, [1.0, 0.9, 0.2], 0.5));
    let expected_over = WorkingColor::premultiplied(std::array::from_fn(|i| {
        overlay.rgba[i] + background.rgba[i] * (1.0 - overlay.alpha())
    }));
    for format in [
        TextureFormat::Rgba8UnormSrgb,
        TextureFormat::Rgba8Unorm,
        TextureFormat::Rgba16Float,
    ] {
        let pixels = render(
            format,
            8,
            4,
            background,
            &[([4.0, 0.0, 8.0, 4.0], overlay, BlendState::PremultipliedOver)],
        );
        let bpp = stellar_renderer_core::color::bytes_per_pixel(format).unwrap();
        let left = encode_pixel(background, format, ColorSpace::Srgb).unwrap();
        let right = encode_pixel(expected_over, format, ColorSpace::Srgb).unwrap();
        assert_pixel(format, &pixels[0..bpp], &left, ColorSpace::Srgb, (0, 0));
        assert_pixel(
            format,
            &pixels[6 * bpp..7 * bpp],
            &right,
            ColorSpace::Srgb,
            (6, 0),
        );
    }
}

#[test]
fn layer_colorspace_follows_format_storage() {
    // Bgra8Unorm은 선형 값을 저장하므로 선형 colorspace를 붙입니다(ADR-0102 결정 2).
    let srgb = ColorSpace::Srgb.tag();
    let p3 = ColorSpace::DisplayP3.tag();
    // SAFETY: CoreGraphics 불변 전역 상수 읽기.
    unsafe {
        let cases = [
            (srgb, TextureFormat::Bgra8UnormSrgb, kCGColorSpaceSRGB),
            (srgb, TextureFormat::Bgra8Unorm, kCGColorSpaceLinearSRGB),
            (
                srgb,
                TextureFormat::Rgba16Float,
                kCGColorSpaceExtendedLinearSRGB,
            ),
            (p3, TextureFormat::Bgra8UnormSrgb, kCGColorSpaceDisplayP3),
            (p3, TextureFormat::Bgra8Unorm, kCGColorSpaceLinearDisplayP3),
            (
                p3,
                TextureFormat::Rgba16Float,
                kCGColorSpaceExtendedLinearDisplayP3,
            ),
        ];
        for (tag, format, expected) in cases {
            assert_eq!(color_space_name(tag, format), Some(expected), "{format:?}");
        }
        assert_eq!(color_space_name(0x7F, TextureFormat::Bgra8Unorm), None);
        assert_eq!(
            color_space_name(srgb, TextureFormat::Depth32FloatStencil8),
            None
        );
    }
}

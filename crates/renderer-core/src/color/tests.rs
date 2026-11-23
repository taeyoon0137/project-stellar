//! 선형 working space·출력 encode 수식 테스트 (XI.10.1).

use super::*;
use crate::backend::{BackendKind, DeviceCapabilities};

/// Display P3 지원 여부만 다른 capability.
pub(crate) fn caps(display_p3: bool) -> DeviceCapabilities {
    DeviceCapabilities {
        backend: BackendKind::Mock,
        max_texture_dimension: 16_384,
        display_p3_surface: display_p3,
        timestamp_query: false,
        msaa_x4: false,
        stencil_bits: 8,
    }
}

fn srgb(r: f32, g: f32, b: f32, a: f32) -> OutputColor {
    OutputColor {
        space: ColorSpace::Srgb,
        rgb: [r, g, b],
        alpha: a,
    }
}

#[track_caller]
fn close(a: [f32; 4], b: [f32; 4], eps: f32) {
    for (x, y) in a.iter().zip(b.iter()) {
        assert!((x - y).abs() <= eps, "{a:?} vs {b:?}");
    }
}

#[test]
fn working_color_is_premultiplied_linear_in_surface_primaries() {
    let plan = OutputColorPlan::negotiate(ColorSpace::Srgb, &caps(false));
    // sRGB 0.5 인코딩 → 선형 0.2140, alpha 0.5를 곱합니다.
    let c = to_working(&plan, srgb(0.5, 1.0, 0.0, 0.5));
    let lin = ColorSpace::Srgb.decode_channel(0.5);
    close(c.rgba, [lin * 0.5, 0.5, 0.0, 0.5], 1e-6);
    close(
        [c.unpremultiplied()[0], c.unpremultiplied()[1], 0.0, 0.0],
        [lin, 1.0, 0.0, 0.0],
        1e-6,
    );
    // 같은 변환은 OutputColorPlan::convert 한 곳입니다(P3 surface면 P3 primaries 선형).
    let p3 = OutputColorPlan::negotiate(ColorSpace::DisplayP3, &caps(true));
    let converted = p3.convert(srgb(1.0, 0.0, 0.0, 1.0));
    close(
        to_working(&p3, srgb(1.0, 0.0, 0.0, 1.0)).rgba,
        [
            converted.linear[0],
            converted.linear[1],
            converted.linear[2],
            1.0,
        ],
        0.0,
    );
}

#[test]
fn image_texels_follow_the_same_conversion_as_solid_colors() {
    let plan = OutputColorPlan::negotiate(ColorSpace::Srgb, &caps(false));
    let texel = image_texel(&plan, ColorSpace::Srgb, [255, 128, 0, 64]);
    let expected = to_working(&plan, srgb(1.0, 128.0 / 255.0, 0.0, 64.0 / 255.0));
    assert_eq!(texel, expected);
}

#[test]
fn output_encoding_table_covers_every_color_format() {
    use OutputEncoding::*;
    let table = [
        (TextureFormat::Rgba8UnormSrgb, Some(TransferEncoded)),
        (TextureFormat::Bgra8UnormSrgb, Some(TransferEncoded)),
        (TextureFormat::Rgba8Unorm, Some(LinearUnorm)),
        (TextureFormat::Bgra8Unorm, Some(LinearUnorm)),
        (TextureFormat::Rgba16Float, Some(LinearFloat)),
        (TextureFormat::R8Unorm, Some(Coverage)),
        (TextureFormat::Depth32FloatStencil8, None),
    ];
    for (format, encoding) in table {
        assert_eq!(output_encoding(format), encoding, "{format:?}");
    }
}

#[test]
fn transfer_encoding_applies_to_premultiplied_channels() {
    let c = WorkingColor::from_straight([0.5, 0.25, 1.0], 0.5);
    let stored = encode_stored(c, OutputEncoding::TransferEncoded, ColorSpace::Srgb);
    close(
        stored,
        [
            ColorSpace::Srgb.encode_channel(0.25),
            ColorSpace::Srgb.encode_channel(0.125),
            ColorSpace::Srgb.encode_channel(0.5),
            0.5,
        ],
        0.0,
    );
    // 선형 unorm은 자르기만, half float는 자르지 않습니다(확장 값 보존, XI.10.6).
    let wide = WorkingColor::premultiplied([1.5, -0.25, 0.5, 1.0]);
    close(
        encode_stored(wide, OutputEncoding::LinearUnorm, ColorSpace::Srgb),
        [1.0, 0.0, 0.5, 1.0],
        0.0,
    );
    close(
        encode_stored(wide, OutputEncoding::LinearFloat, ColorSpace::Srgb),
        [1.5, -0.25, 0.5, 1.0],
        0.0,
    );
    close(
        encode_stored(c, OutputEncoding::Coverage, ColorSpace::Srgb),
        [0.5, 0.0, 0.0, 0.0],
        0.0,
    );
}

#[test]
fn encode_and_decode_pixel_round_trip_within_quantization() {
    let plan = OutputColorPlan::negotiate(ColorSpace::Srgb, &caps(false));
    let color = to_working(&plan, srgb(0.8, 0.3, 0.1, 0.75));
    for format in [
        TextureFormat::Rgba8UnormSrgb,
        TextureFormat::Bgra8UnormSrgb,
        TextureFormat::Rgba8Unorm,
        TextureFormat::Bgra8Unorm,
        TextureFormat::Rgba16Float,
    ] {
        let bytes = encode_pixel(color, format, ColorSpace::Srgb).unwrap();
        assert_eq!(Some(bytes.len()), bytes_per_pixel(format));
        let back = decode_pixel(&bytes, format, ColorSpace::Srgb).unwrap();
        // 8-bit 선형 저장은 어두운 값에서 오차가 가장 큽니다(1 LSB = 1/255 선형).
        close(back.rgba, color.rgba, 1.0 / 255.0 + 1e-6);
    }
    // BGRA는 byte 순서만 다릅니다.
    let rgba = encode_pixel(color, TextureFormat::Rgba8UnormSrgb, ColorSpace::Srgb).unwrap();
    let bgra = encode_pixel(color, TextureFormat::Bgra8UnormSrgb, ColorSpace::Srgb).unwrap();
    assert_eq!(bgra, vec![rgba[2], rgba[1], rgba[0], rgba[3]]);
    assert_eq!(
        decode_pixel(&[1, 2], TextureFormat::Rgba8Unorm, ColorSpace::Srgb),
        None
    );
    assert_eq!(
        encode_pixel(color, TextureFormat::Depth32FloatStencil8, ColorSpace::Srgb),
        None
    );
}

#[test]
fn quantize_rounds_to_nearest_and_clamps() {
    assert_eq!(quantize_unorm8(0.0), 0);
    assert_eq!(quantize_unorm8(1.0), 255);
    assert_eq!(quantize_unorm8(0.5), 128);
    assert_eq!(quantize_unorm8(127.4 / 255.0), 127);
    assert_eq!(quantize_unorm8(-1.0), 0);
    assert_eq!(quantize_unorm8(2.0), 255);
}

#[test]
fn half_float_conversion_matches_ieee_reference_values() {
    let cases: [(f32, u16); 10] = [
        (0.0, 0x0000),
        (-0.0, 0x8000),
        (1.0, 0x3C00),
        (-2.0, 0xC000),
        (0.5, 0x3800),
        (65504.0, 0x7BFF),
        (65520.0, 0x7C00),
        (f32::INFINITY, 0x7C00),
        (5.960_464_5e-8, 0x0001),
        (6.103_515_6e-5, 0x0400),
    ];
    for (value, bits) in cases {
        assert_eq!(f32_to_f16_bits(value), bits, "{value}");
    }
    for bits in [
        0x0000u16, 0x0001, 0x03FF, 0x0400, 0x3555, 0x3C00, 0x7BFF, 0xBC00,
    ] {
        assert_eq!(f32_to_f16_bits(f16_bits_to_f32(bits)), bits);
    }
    assert!(f16_bits_to_f32(f32_to_f16_bits(f32::NAN)).is_nan());
    // round-to-nearest-even: 1 + 2^-11은 1(짝수)로, 1 + 3·2^-11은 1 + 2^-9로.
    assert_eq!(f32_to_f16_bits(1.0 + f32::powi(2.0, -11)), 0x3C00);
    assert_eq!(f32_to_f16_bits(1.0 + 3.0 * f32::powi(2.0, -11)), 0x3C02);
    // 0.1의 half 저장 결과.
    assert!((round_to_f16(0.1) - 0.099_975_586).abs() < 1e-9);
}

#[test]
fn clear_value_carries_working_color() {
    let c = WorkingColor::premultiplied([0.25, 0.5, 0.0, 0.5]);
    let clear = c.to_clear().unwrap();
    assert_eq!(
        [clear.r.get(), clear.g.get(), clear.b.get(), clear.a.get()],
        c.rgba
    );
    assert_eq!(
        WorkingColor::premultiplied([f32::NAN, 0.0, 0.0, 1.0]).to_clear(),
        None
    );
}

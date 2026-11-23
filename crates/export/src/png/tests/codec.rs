//! encode·해석·색 정보·encoder 옵션 테스트 (XX.2.2).

use super::*;

#[test]
fn encode_decode_round_trip_is_lossless_and_deterministic() {
    for profile in OutputProfile::ALL {
        let image = sample_image(profile);
        let bytes = encode_png(&image, 2.0, &CancelToken::new()).unwrap();
        assert_eq!(bytes, encode_png(&image, 2.0, &CancelToken::new()).unwrap());
        let decoded = decode_png(&bytes, DecodeLimits::default()).unwrap();
        assert_eq!((decoded.width, decoded.height), (4, 3));
        assert_eq!((decoded.bit_depth, decoded.color_type), (8, 6));
        assert_eq!(decoded.rgba, image.pixels());
        // 배율 2 = 144 dpi = 5669 pixel/m.
        assert_eq!(decoded.color.physical, Some((5669, 5669, 1)));
        assert_eq!(decoded.chunks.first(), Some(b"IHDR"));
        assert_eq!(decoded.chunks.last(), Some(b"IEND"));
        assert!(!decoded.chunks.contains(b"tIME"));
    }
}

#[test]
fn srgb_and_display_p3_metadata_are_recorded() {
    let srgb_png = decode_png(
        &encode_png(&sample_image(OutputProfile::Srgb), 1.0, &CancelToken::new()).unwrap(),
        DecodeLimits::default(),
    )
    .unwrap();
    assert_eq!(srgb_png.color.srgb_intent, Some(0));
    assert_eq!(srgb_png.color.gamma, Some(45_455));
    assert_eq!(srgb_png.color.cicp, None);
    assert_eq!(srgb_png.color.icc, None);
    assert_eq!(srgb_png.color.color_space(), Ok(ColorSpace::Srgb));
    assert_eq!(
        srgb_png.chunks,
        vec![*b"IHDR", *b"sRGB", *b"gAMA", *b"cHRM", *b"pHYs", *b"IDAT", *b"IEND"]
    );
    let p3_png = decode_png(
        &encode_png(
            &sample_image(OutputProfile::DisplayP3),
            1.0,
            &CancelToken::new(),
        )
        .unwrap(),
        DecodeLimits::default(),
    )
    .unwrap();
    assert_eq!(
        p3_png.color.cicp,
        Some(Cicp {
            primaries: 12,
            transfer: 13,
            matrix: 0,
            full_range: 1
        })
    );
    let (name, profile) = p3_png.color.icc.clone().unwrap();
    assert_eq!(name, "Display P3");
    assert_eq!(profile, icc::display_p3_profile());
    assert_eq!(icc::primaries_space(&profile), Some(ColorSpace::DisplayP3));
    assert_eq!(
        p3_png.color.srgb_intent, None,
        "iCCP와 sRGB는 함께 쓰지 않음"
    );
    assert_eq!(p3_png.color.color_space(), Ok(ColorSpace::DisplayP3));
    // cICP가 없어도 ICC colorant로 P3를 판정합니다.
    let icc_only = PngColorInfo {
        icc: Some(("x".to_string(), profile)),
        ..PngColorInfo::default()
    };
    assert_eq!(icc_only.color_space(), Ok(ColorSpace::DisplayP3));
    let bt2020 = PngColorInfo {
        cicp: Some(Cicp {
            primaries: 9,
            transfer: 13,
            matrix: 0,
            full_range: 1,
        }),
        ..PngColorInfo::default()
    };
    assert!(bt2020.color_space().is_err());
}

#[test]
fn unsupported_encoder_options_are_errors() {
    assert_eq!(
        PngOptions::from_names("display-p3", 8).map(|o| o.profile()),
        Ok(OutputProfile::DisplayP3)
    );
    assert!(matches!(
        PngOptions::from_names("rec2020", 8),
        Err(PngOptionsError::UnsupportedProfile(_))
    ));
    assert_eq!(
        PngOptions::from_names("srgb", 16),
        Err(PngOptionsError::UnsupportedBitDepth { bits: 16 })
    );
    assert_eq!(
        PngOptions::from_names("srgb", 12),
        Err(PngOptionsError::UnsupportedBitDepth { bits: 12 })
    );
    assert_eq!(
        PngOptions::new(OutputProfile::Srgb, PngBitDepth::Sixteen),
        Err(PngOptionsError::UnsupportedBitDepth { bits: 16 })
    );
}

/// 참조 구현(Python zlib + 손 PNG writer)이 만든 PNG 변형.
const GRAY16_TRNS: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x10, 0x00, 0x00, 0x00, 0x00, 0x81, 0xD9, 0xFC,
    0x15, 0x00, 0x00, 0x00, 0x02, 0x74, 0x52, 0x4E, 0x53, 0x00, 0x00, 0x76, 0x93, 0xCD, 0x38, 0x00,
    0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x60, 0x60, 0xF8, 0xFF, 0x1F, 0x00,
    0x03, 0x02, 0x01, 0xFF, 0x39, 0x29, 0x19, 0xBE, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];
const PALETTE4_TRNS: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x04, 0x03, 0x00, 0x00, 0x00, 0xE9, 0xCE, 0x09,
    0x87, 0x00, 0x00, 0x00, 0x09, 0x50, 0x4C, 0x54, 0x45, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00,
    0x00, 0xFF, 0x2D, 0x4A, 0xCD, 0x8A, 0x00, 0x00, 0x00, 0x02, 0x74, 0x52, 0x4E, 0x53, 0xFF, 0x80,
    0x08, 0x0F, 0xB3, 0x6A, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x60,
    0x54, 0x00, 0x00, 0x00, 0x25, 0x00, 0x22, 0xE9, 0x82, 0x87, 0xE5, 0x00, 0x00, 0x00, 0x00, 0x49,
    0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];
const RGB_SUB_UP: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xFD, 0xD4, 0x9A,
    0x73, 0x00, 0x00, 0x00, 0x13, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xE4, 0x12, 0x91, 0x63,
    0x65, 0x65, 0x65, 0x02, 0x02, 0x20, 0x05, 0x00, 0x03, 0xA9, 0x00, 0x64, 0xBE, 0x35, 0x36, 0x8B,
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[test]
fn decodes_reference_png_variants() {
    let gray = decode_png(GRAY16_TRNS, DecodeLimits::default()).unwrap();
    assert_eq!(gray.rgba, vec![0, 0, 0, 0, 255, 255, 255, 255]);
    let palette = decode_png(PALETTE4_TRNS, DecodeLimits::default()).unwrap();
    assert_eq!(
        palette.rgba,
        vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255]
    );
    let rgb = decode_png(RGB_SUB_UP, DecodeLimits::default()).unwrap();
    assert_eq!(
        rgb.rgba,
        vec![10, 20, 30, 255, 15, 25, 35, 255, 12, 22, 32, 255, 20, 30, 40, 255]
    );
}

#[test]
fn decoder_rejects_corrupt_and_unsupported_input() {
    assert_eq!(
        decode_png(b"not a png", DecodeLimits::default()),
        Err(PngDecodeError::NotPng)
    );
    let mut bad_crc = RGB_SUB_UP.to_vec();
    bad_crc[20] ^= 1;
    assert!(matches!(
        decode_png(&bad_crc, DecodeLimits::default()),
        Err(PngDecodeError::Crc { .. })
    ));
    assert_eq!(
        decode_png(&RGB_SUB_UP[..40], DecodeLimits::default()),
        Err(PngDecodeError::Truncated)
    );
    let tiny = DecodeLimits {
        max_pixels: 2,
        max_inflated: 1 << 20,
    };
    assert_eq!(decode_png(RGB_SUB_UP, tiny), Err(PngDecodeError::TooLarge));
    // interlace bit를 켜고 CRC를 다시 맞춘 IHDR은 미지원입니다.
    let mut interlaced = RGB_SUB_UP.to_vec();
    interlaced[28] = 1;
    let crc = zlib::crc32(&interlaced[12..29]);
    interlaced[29..33].copy_from_slice(&crc.to_be_bytes());
    assert!(matches!(
        decode_png(&interlaced, DecodeLimits::default()),
        Err(PngDecodeError::Unsupported { .. })
    ));
    // 임의 변형은 panic 없이 오류이거나 성공합니다.
    let good = encode_png(&sample_image(OutputProfile::Srgb), 1.0, &CancelToken::new()).unwrap();
    for i in 8..good.len() {
        let mut copy = good.clone();
        copy[i] ^= 0x5A;
        let _ = decode_png(&copy, DecodeLimits::default());
    }
}

#[test]
fn encode_stops_when_cancelled() {
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(encode_png(&sample_image(OutputProfile::Srgb), 1.0, &cancel).is_err());
}

//! glyph key·interface 테스트 (XI.6.4).

use super::*;
use std::collections::HashSet;

use stellar_scene::geometry::PathCommand;
use stellar_scene::Vec2;

pub(crate) fn font(seed: u8, face: u32) -> FontHash {
    let mut digest = [0u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = seed.wrapping_mul(31).wrapping_add(index as u8);
    }
    FontHash::new(digest, face)
}

fn tag(name: &str) -> AxisTag {
    AxisTag::parse(name).unwrap()
}

pub(crate) fn key(
    font: FontHash,
    glyph: u32,
    px: f32,
    variations: VariationCoords,
    mode: GlyphRenderMode,
) -> GlyphKey {
    GlyphKey {
        font,
        glyph: GlyphId(glyph),
        size: GlyphSize::from_px(px).unwrap(),
        variations,
        mode,
    }
}

#[test]
fn key_fixture_has_no_collisions() {
    // 검증 기준(XI.6.4): key collision fixture 없음.
    // 각 성분만 다른 key가 서로 다르고, canonical bytes와 FNV digest도 모두 다릅니다.
    let fonts = [
        font(1, 0),
        font(1, 1),
        font(2, 0),
        font(3, 0),
        // digest 첫 byte만 다른 폰트.
        {
            let mut digest = *font(1, 0).file_sha256();
            digest[0] ^= 1;
            FontHash::new(digest, 0)
        },
    ];
    let variation_sets = [
        VariationCoords::none(),
        VariationCoords::new([(tag("wght"), 400.0)]).unwrap(),
        VariationCoords::new([(tag("wght"), 700.0)]).unwrap(),
        VariationCoords::new([(tag("wdth"), 400.0)]).unwrap(),
        VariationCoords::new([(tag("wght"), 400.0), (tag("wdth"), 100.0)]).unwrap(),
        // 값 bits는 같고 tag 경계만 다른 조합(길이 prefix가 단사성을 지킵니다).
        VariationCoords::new([(tag("wdth"), 100.0), (tag("opsz"), 400.0)]).unwrap(),
    ];
    let sizes = [8.0f32, 12.0, 12.015_625, 13.0, 16.0, 24.0, 96.0, 4096.0];
    let modes = [
        GlyphRenderMode::Outline,
        GlyphRenderMode::Coverage,
        GlyphRenderMode::Color,
    ];
    let mut keys = HashSet::new();
    let mut bytes = HashSet::new();
    let mut digests = HashSet::new();
    let mut count = 0usize;
    for font in fonts {
        for glyph in (0..40u32).chain([255, 256, 65_535, 65_536, u32::MAX]) {
            for px in sizes {
                for variations in &variation_sets {
                    for mode in modes {
                        let k = key(font, glyph, px, variations.clone(), mode);
                        count += 1;
                        assert!(bytes.insert(k.canonical_bytes()), "bytes 충돌: {k:?}");
                        assert!(digests.insert(k.digest()), "digest 충돌: {k:?}");
                        assert!(keys.insert(k));
                    }
                }
            }
        }
    }
    assert_eq!(count, 5 * 45 * 8 * 6 * 3);
    assert_eq!(keys.len(), count);
}

#[test]
fn equivalent_inputs_share_one_key() {
    let f = font(7, 0);
    // variation 순서·중복(마지막 값 우선)·-0.0은 같은 정규형.
    let a = VariationCoords::new([(tag("wght"), 500.0), (tag("wdth"), 0.0)]).unwrap();
    let b = VariationCoords::new([
        (tag("wdth"), -0.0),
        (tag("wght"), 100.0),
        (tag("wght"), 500.0),
    ])
    .unwrap();
    assert_eq!(a, b);
    assert_eq!(
        a.axes().collect::<Vec<_>>(),
        vec![(tag("wdth"), 0.0), (tag("wght"), 500.0)]
    );
    let ka = key(f, 10, 16.0, a, GlyphRenderMode::Coverage);
    let kb = key(f, 10, 16.0, b, GlyphRenderMode::Coverage);
    assert_eq!(ka, kb);
    assert_eq!(ka.canonical_bytes(), kb.canonical_bytes());
    // 1/64 px 미만 차이는 같은 크기, 1/64 px 차이는 다른 크기.
    assert_eq!(
        GlyphSize::from_px(16.0).unwrap(),
        GlyphSize::from_px(16.004).unwrap()
    );
    assert_ne!(
        GlyphSize::from_px(16.0).unwrap(),
        GlyphSize::from_px(16.015_625).unwrap()
    );
    assert_eq!(GlyphSize::from_px(16.0).unwrap().sixty_fourths(), 1024);
    assert_eq!(GlyphSize::from_px(16.0).unwrap().px(), 16.0);
    // 크기만 다르면 같은 glyph 모양.
    let small = key(
        f,
        10,
        8.0,
        VariationCoords::none(),
        GlyphRenderMode::Coverage,
    );
    let large = key(
        f,
        10,
        32.0,
        VariationCoords::none(),
        GlyphRenderMode::Coverage,
    );
    assert!(small.same_glyph_shape(&large));
    let other_glyph = key(
        f,
        11,
        8.0,
        VariationCoords::none(),
        GlyphRenderMode::Coverage,
    );
    assert!(!small.same_glyph_shape(&other_glyph));
    let other_font = key(
        font(8, 0),
        10,
        8.0,
        VariationCoords::none(),
        GlyphRenderMode::Coverage,
    );
    assert!(!small.same_glyph_shape(&other_font));
}

#[test]
fn canonical_bytes_layout_is_stable() {
    let k = key(
        FontHash::new([0xAB; 32], 2),
        0x0102_0304,
        1.0,
        VariationCoords::new([(tag("wght"), 1.0)]).unwrap(),
        GlyphRenderMode::Color,
    );
    let mut expected = vec![1u8];
    expected.extend_from_slice(&[0xAB; 32]);
    expected.extend_from_slice(&2u32.to_le_bytes());
    expected.extend_from_slice(&0x0102_0304u32.to_le_bytes());
    expected.extend_from_slice(&64u32.to_le_bytes());
    expected.push(2);
    expected.extend_from_slice(&1u16.to_le_bytes());
    expected.extend_from_slice(b"wght");
    expected.extend_from_slice(&1.0f32.to_bits().to_le_bytes());
    assert_eq!(k.canonical_bytes(), expected);
}

#[test]
fn untrusted_values_are_rejected() {
    for bad in [0.0, -1.0, 0.001, f32::NAN, f32::INFINITY, 4096.5] {
        assert_eq!(
            GlyphSize::from_px(bad),
            Err(GlyphError::InvalidSize),
            "{bad}"
        );
    }
    assert!(GlyphSize::from_px(4096.0).is_ok());
    assert_eq!(
        GlyphSize::from_sixty_fourths(0),
        Err(GlyphError::InvalidSize)
    );
    assert_eq!(
        GlyphSize::from_sixty_fourths(4096 * 64 + 1),
        Err(GlyphError::InvalidSize)
    );
    assert_eq!(AxisTag::parse("wgh"), Err(GlyphError::InvalidTag));
    assert_eq!(
        AxisTag::new([b'w', 0x7F, b'h', b't']),
        Err(GlyphError::InvalidTag)
    );
    assert_eq!(
        VariationCoords::new([(tag("wght"), f32::NAN)]),
        Err(GlyphError::NonFiniteVariation)
    );
    let too_many = (0..=MAX_VARIATION_AXES as u8).map(|i| {
        (
            AxisTag::new([b'a', b'a' + i / 26, b'a' + i % 26, b'x']).unwrap(),
            1.0,
        )
    });
    assert!(matches!(
        VariationCoords::new(too_many),
        Err(GlyphError::TooManyAxes { .. })
    ));
    // 같은 tag 반복으로 무한히 긴 입력도 상한에서 멈춥니다.
    assert!(matches!(
        VariationCoords::new(std::iter::repeat((tag("wght"), 1.0))),
        Err(GlyphError::TooManyAxes { .. })
    ));

    // bitmap 길이·크기 검증.
    assert!(GlyphBitmap::new(2, 2, 0, -2, GlyphBitmapFormat::Coverage8, vec![0; 4]).is_ok());
    assert!(GlyphBitmap::new(
        2,
        2,
        0,
        0,
        GlyphBitmapFormat::Rgba8Premultiplied,
        vec![0; 4]
    )
    .is_err());
    assert!(GlyphBitmap::new(0, 2, 0, 0, GlyphBitmapFormat::Coverage8, vec![]).is_err());
    assert!(GlyphBitmap::new(
        MAX_GLYPH_BITMAP_DIM + 1,
        1,
        0,
        0,
        GlyphBitmapFormat::Coverage8,
        vec![0; MAX_GLYPH_BITMAP_DIM as usize + 1]
    )
    .is_err());
}

/// 테스트 rasterizer: Outline mode는 사각형 path, 나머지는 크기만큼의 bitmap. glyph 0(공백)은 Empty.
struct BoxRasterizer;

impl GlyphRasterizer for BoxRasterizer {
    fn rasterize(&self, key: &GlyphKey) -> Result<GlyphImage, GlyphError> {
        if key.glyph == GlyphId(0) {
            return Ok(GlyphImage::Empty);
        }
        let side = (key.size.px().ceil() as u32).max(1);
        match key.mode.bitmap_format() {
            None => {
                let s = key.size.px();
                let p = |x: f32, y: f32| Vec2::new(x, y).unwrap();
                Ok(GlyphImage::Outline(
                    PathGeometry::new(vec![
                        PathCommand::MoveTo(p(0.0, -s)),
                        PathCommand::LineTo(p(s, -s)),
                        PathCommand::LineTo(p(s, 0.0)),
                        PathCommand::Close,
                    ])
                    .unwrap(),
                ))
            }
            Some(format) => Ok(GlyphImage::Bitmap(GlyphBitmap::new(
                side,
                side,
                0,
                -(side as i32),
                format,
                vec![0xFF; (side * side) as usize * format.bytes_per_pixel()],
            )?)),
        }
    }
}

#[test]
fn rasterizer_output_must_match_mode() {
    let rasterizer = BoxRasterizer;
    for mode in [
        GlyphRenderMode::Outline,
        GlyphRenderMode::Coverage,
        GlyphRenderMode::Color,
    ] {
        let k = key(font(1, 0), 5, 12.0, VariationCoords::none(), mode);
        let image = rasterizer.rasterize(&k).unwrap();
        assert!(check_image_matches_mode(&k, &image).is_ok());
        // Empty는 모든 mode에 허용.
        let blank = GlyphKey {
            glyph: GlyphId(0),
            ..k.clone()
        };
        assert_eq!(rasterizer.rasterize(&blank).unwrap(), GlyphImage::Empty);
        // 다른 mode로 만든 결과는 거부합니다.
        let other = GlyphKey {
            mode: if mode == GlyphRenderMode::Outline {
                GlyphRenderMode::Coverage
            } else {
                GlyphRenderMode::Outline
            },
            ..k.clone()
        };
        assert!(check_image_matches_mode(&other, &image).is_err());
    }
    let coverage = key(
        font(1, 0),
        5,
        12.0,
        VariationCoords::none(),
        GlyphRenderMode::Coverage,
    );
    let color_bitmap = GlyphImage::Bitmap(
        GlyphBitmap::new(
            1,
            1,
            0,
            0,
            GlyphBitmapFormat::Rgba8Premultiplied,
            vec![0; 4],
        )
        .unwrap(),
    );
    assert!(check_image_matches_mode(&coverage, &color_bitmap).is_err());
}

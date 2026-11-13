//! missing/decoding asset placeholder (XI.7.4).
//!
//! asset이 없거나(VI.8.2 missing resource) 아직 decode 중인 Image는 node의 layout bounds를 그대로
//! 유지한 채 **결정론적 checker**로 그립니다.
//!
//! - **공유 데이터**: texture는 이 모듈이 코드(컴파일 시점 `const fn`)로 생성하는 tile 하나뿐입니다.
//!   Metal·D3D12 backend는 자체 placeholder를 만들지 않고 [`placeholder_texture`]가 반환하는 같은
//!   `&'static` pixel과 [`SamplerDesc`]를 그대로 upload합니다.
//! - **tile**: 16×16 px, [`DecodedPixelFormat::Rgba8Unorm`](XI.7.1), sRGB, 불투명. 8 px cell의
//!   2×2 checker이며 `x / 8 + y / 8`이 짝수인 cell이 light, 홀수인 cell이 dark입니다.
//!   [`PlaceholderKind`]마다 neutral gray 한 쌍을 씁니다(Missing은 대비가 크고 Decoding은 옅음).
//! - **배치**: checker는 node local 원점에 고정하고 cell 한 변은 local 8 단위입니다. quad는 layout
//!   bounds 전체를 덮고 uv = local 좌표 / 16, sampler는 Nearest + Repeat, mipmap 없음입니다. pan해도
//!   무늬가 흐르지 않고 zoom에 비례합니다.
//! - asset 크기를 알 수 없으므로 fit·crop은 적용하지 않습니다. decode 실패(손상 asset)는 Missing
//!   placeholder로 표시합니다.
//! - [`rasterize_placeholder_reference`]는 1:1 배율 CPU 기준 screenshot입니다. backend screenshot
//!   적합성 테스트는 이 결과와 비교합니다.

use std::fmt;

use stellar_scene::{ColorSpace, Size2};

use crate::image_decode::{DecodeLimits, DecodedImage, DecodedPixelFormat, ImageHeader};

/// checker cell 한 변(local 단위 = tile pixel).
pub const PLACEHOLDER_CELL: u32 = 8;

/// tile 한 변(pixel). cell 2개.
pub const PLACEHOLDER_TILE: u32 = PLACEHOLDER_CELL * 2;

const TILE_BYTES: usize = (PLACEHOLDER_TILE * PLACEHOLDER_TILE * 4) as usize;

/// placeholder 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaceholderKind {
    /// asset 누락 또는 decode 실패.
    Missing,
    /// decode 진행 중.
    Decoding,
}

/// checker 색 한 쌍(sRGB 인코딩 RGBA8, straight alpha).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlaceholderColors {
    /// 짝수 cell 색.
    pub light: [u8; 4],
    /// 홀수 cell 색.
    pub dark: [u8; 4],
}

const MISSING_COLORS: PlaceholderColors = PlaceholderColors {
    light: [0xCC, 0xCC, 0xCC, 0xFF],
    dark: [0x99, 0x99, 0x99, 0xFF],
};

const DECODING_COLORS: PlaceholderColors = PlaceholderColors {
    light: [0xF0, 0xF0, 0xF0, 0xFF],
    dark: [0xE0, 0xE0, 0xE0, 0xFF],
};

impl PlaceholderKind {
    /// checker 색.
    pub const fn colors(self) -> PlaceholderColors {
        match self {
            PlaceholderKind::Missing => MISSING_COLORS,
            PlaceholderKind::Decoding => DECODING_COLORS,
        }
    }
}

/// Image asset의 렌더 가능 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageAvailability {
    /// decode 완료, texture 사용 가능.
    Ready,
    /// decode 진행 중.
    Decoding,
    /// asset 누락(VI.8.2 missing resource).
    Missing,
    /// asset은 있으나 decode 실패.
    DecodeFailed,
}

impl ImageAvailability {
    /// 그려야 할 placeholder. `Ready`면 `None`.
    pub const fn placeholder(self) -> Option<PlaceholderKind> {
        match self {
            ImageAvailability::Ready => None,
            ImageAvailability::Decoding => Some(PlaceholderKind::Decoding),
            ImageAvailability::Missing | ImageAvailability::DecodeFailed => {
                Some(PlaceholderKind::Missing)
            }
        }
    }
}

/// texture filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFilter {
    /// 최근접.
    Nearest,
    /// 선형.
    Linear,
}

/// texture 좌표 wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureWrap {
    /// 가장자리 고정.
    ClampToEdge,
    /// 반복.
    Repeat,
}

/// backend 공통 sampler 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerDesc {
    /// 확대·축소 filter.
    pub filter: TextureFilter,
    /// u·v wrap.
    pub wrap: TextureWrap,
    /// mipmap 사용 여부.
    pub mipmaps: bool,
}

/// placeholder sampler: Nearest + Repeat, mipmap 없음.
pub const PLACEHOLDER_SAMPLER: SamplerDesc = SamplerDesc {
    filter: TextureFilter::Nearest,
    wrap: TextureWrap::Repeat,
    mipmaps: false,
};

/// 양 backend가 공유하는 placeholder tile texture.
#[derive(Debug, PartialEq, Eq)]
pub struct PlaceholderTexture {
    kind: PlaceholderKind,
    pixels: [u8; TILE_BYTES],
}

const fn build_tile(colors: PlaceholderColors) -> [u8; TILE_BYTES] {
    let mut out = [0u8; TILE_BYTES];
    let mut y = 0;
    while y < PLACEHOLDER_TILE {
        let mut x = 0;
        while x < PLACEHOLDER_TILE {
            let color = if (x / PLACEHOLDER_CELL + y / PLACEHOLDER_CELL).is_multiple_of(2) {
                colors.light
            } else {
                colors.dark
            };
            let offset = ((y * PLACEHOLDER_TILE + x) * 4) as usize;
            out[offset] = color[0];
            out[offset + 1] = color[1];
            out[offset + 2] = color[2];
            out[offset + 3] = color[3];
            x += 1;
        }
        y += 1;
    }
    out
}

static MISSING_TEXTURE: PlaceholderTexture = PlaceholderTexture {
    kind: PlaceholderKind::Missing,
    pixels: build_tile(MISSING_COLORS),
};

static DECODING_TEXTURE: PlaceholderTexture = PlaceholderTexture {
    kind: PlaceholderKind::Decoding,
    pixels: build_tile(DECODING_COLORS),
};

/// 종류별 공유 placeholder texture. 같은 종류는 항상 같은 `&'static` 데이터입니다.
pub fn placeholder_texture(kind: PlaceholderKind) -> &'static PlaceholderTexture {
    match kind {
        PlaceholderKind::Missing => &MISSING_TEXTURE,
        PlaceholderKind::Decoding => &DECODING_TEXTURE,
    }
}

impl PlaceholderTexture {
    /// 종류.
    pub fn kind(&self) -> PlaceholderKind {
        self.kind
    }

    /// 너비(pixel).
    pub fn width(&self) -> u32 {
        PLACEHOLDER_TILE
    }

    /// 높이(pixel).
    pub fn height(&self) -> u32 {
        PLACEHOLDER_TILE
    }

    /// pixel format.
    pub fn format(&self) -> DecodedPixelFormat {
        DecodedPixelFormat::Rgba8Unorm
    }

    /// RGB 인코딩 색공간.
    pub fn color_space(&self) -> ColorSpace {
        ColorSpace::Srgb
    }

    /// sampler 설정.
    pub fn sampler(&self) -> SamplerDesc {
        PLACEHOLDER_SAMPLER
    }

    /// tile pixel bytes(row-major, 좌상단 원점).
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Nearest + Repeat로 `(u, v)`를 sampling합니다(GPU sampler와 같은 규칙).
    pub fn sample(&self, u: f64, v: f64) -> [u8; 4] {
        let texel = |t: f64| -> usize {
            let fraction = t - t.floor();
            let index = (fraction * f64::from(PLACEHOLDER_TILE)).floor();
            // fraction ∈ [0, 1)이지만 반올림 경계를 위해 상한을 고정합니다.
            (index.max(0.0) as usize).min(PLACEHOLDER_TILE as usize - 1)
        };
        let offset = (texel(v) * PLACEHOLDER_TILE as usize + texel(u)) * 4;
        [
            self.pixels[offset],
            self.pixels[offset + 1],
            self.pixels[offset + 2],
            self.pixels[offset + 3],
        ]
    }
}

/// layout bounds를 덮는 placeholder quad. local 원점 `(0, 0)`의 uv는 `(0, 0)`입니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaceholderQuad {
    /// quad 크기 = node layout bounds 크기.
    pub size: Size2,
    /// `(width, height)` 꼭짓점의 uv = 크기 / tile.
    pub uv_max: [f32; 2],
}

/// layout bounds 크기에 대한 placeholder quad. bounds는 asset 유무와 무관하게 그대로 유지합니다.
pub fn placeholder_quad(bounds: Size2) -> PlaceholderQuad {
    let tile = PLACEHOLDER_TILE as f32;
    PlaceholderQuad {
        size: bounds,
        uv_max: [bounds.width() / tile, bounds.height() / tile],
    }
}

/// 기준 screenshot 생성 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceholderError {
    /// 너비 또는 높이가 0입니다.
    ZeroSize,
    /// 크기가 decode 상한([`DecodeLimits::DEFAULT`])을 넘습니다.
    TooLarge,
}

impl fmt::Display for PlaceholderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaceholderError::ZeroSize => f.write_str("placeholder 크기는 0일 수 없습니다"),
            PlaceholderError::TooLarge => f.write_str("placeholder 크기가 상한을 넘습니다"),
        }
    }
}

impl std::error::Error for PlaceholderError {}

/// `width × height` px bounds에 1:1 배율로 그린 placeholder의 CPU 기준 screenshot.
///
/// 각 pixel 중심 `(x + 0.5, y + 0.5)`의 uv(= local / 16)를 [`PlaceholderTexture::sample`]로
/// sampling합니다. 결과는 XI.7.1 decoded format(RGBA8, sRGB)입니다.
pub fn rasterize_placeholder_reference(
    kind: PlaceholderKind,
    width: u32,
    height: u32,
) -> Result<DecodedImage, PlaceholderError> {
    if width == 0 || height == 0 {
        return Err(PlaceholderError::ZeroSize);
    }
    DecodeLimits::DEFAULT
        .check_header(&ImageHeader {
            width,
            height,
            color_space: ColorSpace::Srgb,
        })
        .map_err(|_| PlaceholderError::TooLarge)?;
    let texture = placeholder_texture(kind);
    let tile = f64::from(PLACEHOLDER_TILE);
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        let v = (f64::from(y) + 0.5) / tile;
        for x in 0..width {
            let u = (f64::from(x) + 0.5) / tile;
            pixels.extend_from_slice(&texture.sample(u, v));
        }
    }
    DecodedImage::new(width, height, ColorSpace::Srgb, pixels)
        .map_err(|_| PlaceholderError::TooLarge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::fnv1a64;
    use std::thread;

    /// 20×12 px 기준 screenshot fixture. `.` = light, `#` = dark.
    const FIXTURE_20X12: [&str; 12] = [
        "........########....",
        "........########....",
        "........########....",
        "........########....",
        "........########....",
        "........########....",
        "........########....",
        "........########....",
        "########........####",
        "########........####",
        "########........####",
        "########........####",
    ];

    /// 40×24 px 기준 screenshot의 cell(8 px) map.
    const FIXTURE_40X24_CELLS: [&str; 3] = ["LDLDL", "DLDLD", "LDLDL"];

    /// 기준 screenshot bytes의 FNV-1a 64 golden.
    const GOLDEN_DIGESTS: [(PlaceholderKind, u32, u32, u64); 4] = [
        (PlaceholderKind::Missing, 20, 12, 0x7018_d542_b025_7fa5),
        (PlaceholderKind::Decoding, 20, 12, 0xa980_c9c9_571a_42a5),
        (PlaceholderKind::Missing, 40, 24, 0x9105_27aa_8d06_fd25),
        (PlaceholderKind::Decoding, 40, 24, 0x6fdf_f2b1_d8ac_7925),
    ];

    /// fixture 문자를 fixture에 적힌 색 literal로 바꿔 기대 screenshot을 만듭니다(구현과 독립).
    fn expected_from_pixels(rows: &[&str], light: [u8; 4], dark: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        for row in rows {
            for c in row.chars() {
                out.extend_from_slice(if c == '.' { &light } else { &dark });
            }
        }
        out
    }

    fn expected_from_cells(rows: &[&str], light: [u8; 4], dark: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        for row in rows {
            for _ in 0..8 {
                for c in row.chars() {
                    for _ in 0..8 {
                        out.extend_from_slice(if c == 'L' { &light } else { &dark });
                    }
                }
            }
        }
        out
    }

    const MISSING_LIGHT: [u8; 4] = [0xCC, 0xCC, 0xCC, 0xFF];
    const MISSING_DARK: [u8; 4] = [0x99, 0x99, 0x99, 0xFF];
    const DECODING_LIGHT: [u8; 4] = [0xF0, 0xF0, 0xF0, 0xFF];
    const DECODING_DARK: [u8; 4] = [0xE0, 0xE0, 0xE0, 0xFF];

    #[test]
    fn reference_screenshot_matches_fixture() {
        // 검증 기준(XI.7.4): screenshot fixture 동일.
        for (kind, light, dark) in [
            (PlaceholderKind::Missing, MISSING_LIGHT, MISSING_DARK),
            (PlaceholderKind::Decoding, DECODING_LIGHT, DECODING_DARK),
        ] {
            let small = rasterize_placeholder_reference(kind, 20, 12).unwrap();
            assert_eq!(small.format(), DecodedPixelFormat::Rgba8Unorm);
            assert_eq!(small.color_space(), ColorSpace::Srgb);
            assert_eq!(
                small.pixels(),
                expected_from_pixels(&FIXTURE_20X12, light, dark).as_slice(),
                "{kind:?} 20x12"
            );
            let large = rasterize_placeholder_reference(kind, 40, 24).unwrap();
            assert_eq!(
                large.pixels(),
                expected_from_cells(&FIXTURE_40X24_CELLS, light, dark).as_slice(),
                "{kind:?} 40x24"
            );
        }
        for (kind, width, height, digest) in GOLDEN_DIGESTS {
            let image = rasterize_placeholder_reference(kind, width, height).unwrap();
            assert_eq!(fnv1a64(image.pixels()), digest, "{kind:?} {width}x{height}");
        }
    }

    #[test]
    fn both_backends_share_identical_placeholder_data() {
        // Metal·D3D12 경로가 각자 thread에서 placeholder를 요청해도 같은 static 데이터와 sampler.
        for kind in [PlaceholderKind::Missing, PlaceholderKind::Decoding] {
            let fetch = move || {
                let texture = placeholder_texture(kind);
                (
                    texture as *const PlaceholderTexture as usize,
                    texture.pixels().to_vec(),
                    texture.sampler(),
                    rasterize_placeholder_reference(kind, 33, 17).unwrap(),
                )
            };
            let metal = thread::Builder::new()
                .name("metal".into())
                .spawn(fetch)
                .unwrap()
                .join()
                .unwrap();
            let d3d12 = thread::Builder::new()
                .name("d3d12".into())
                .spawn(fetch)
                .unwrap()
                .join()
                .unwrap();
            assert_eq!(metal.0, d3d12.0, "같은 static 주소");
            assert_eq!(metal.1, d3d12.1, "tile bytes 동일");
            assert_eq!(metal.2, d3d12.2, "sampler 동일");
            assert_eq!(metal.3, d3d12.3, "기준 screenshot 동일");
        }
    }

    #[test]
    fn tile_texture_contract() {
        for kind in [PlaceholderKind::Missing, PlaceholderKind::Decoding] {
            let texture = placeholder_texture(kind);
            assert_eq!(texture.kind(), kind);
            assert_eq!((texture.width(), texture.height()), (16, 16));
            assert_eq!(texture.format(), DecodedPixelFormat::Rgba8Unorm);
            assert_eq!(texture.color_space(), ColorSpace::Srgb);
            assert_eq!(texture.pixels().len(), 16 * 16 * 4);
            assert_eq!(texture.sampler(), PLACEHOLDER_SAMPLER);
            assert_eq!(
                PLACEHOLDER_SAMPLER,
                SamplerDesc {
                    filter: TextureFilter::Nearest,
                    wrap: TextureWrap::Repeat,
                    mipmaps: false,
                }
            );
            let colors = kind.colors();
            // 모든 pixel 불투명, cell parity 규칙.
            for y in 0..16u32 {
                for x in 0..16u32 {
                    let offset = ((y * 16 + x) * 4) as usize;
                    let pixel = &texture.pixels()[offset..offset + 4];
                    let expected = if (x / 8 + y / 8) % 2 == 0 {
                        colors.light
                    } else {
                        colors.dark
                    };
                    assert_eq!(pixel, expected, "{kind:?} ({x}, {y})");
                    assert_eq!(pixel[3], 0xFF);
                }
            }
        }
        assert_ne!(
            placeholder_texture(PlaceholderKind::Missing).pixels(),
            placeholder_texture(PlaceholderKind::Decoding).pixels()
        );
    }

    #[test]
    fn repeat_sampling_and_quad_uv() {
        let texture = placeholder_texture(PlaceholderKind::Missing);
        // Repeat: 정수 tile 이동·음수 좌표에서 같은 texel.
        assert_eq!(texture.sample(0.03, 0.03), texture.sample(3.03, -1.97));
        assert_eq!(texture.sample(0.03, 0.03), MISSING_LIGHT);
        assert_eq!(texture.sample(0.53, 0.03), MISSING_DARK);
        assert_eq!(texture.sample(0.999_999, 0.999_999), MISSING_LIGHT);

        // quad는 layout bounds를 그대로 쓰고 uv = 크기 / 16.
        let quad = placeholder_quad(Size2::new(40.0, 24.0).unwrap());
        assert_eq!(quad.size, Size2::new(40.0, 24.0).unwrap());
        assert_eq!(quad.uv_max, [2.5, 1.5]);
        let odd = placeholder_quad(Size2::new(10.0, 3.0).unwrap());
        assert_eq!(odd.uv_max, [0.625, 0.1875]);
    }

    #[test]
    fn availability_maps_to_placeholder() {
        assert_eq!(ImageAvailability::Ready.placeholder(), None);
        assert_eq!(
            ImageAvailability::Decoding.placeholder(),
            Some(PlaceholderKind::Decoding)
        );
        assert_eq!(
            ImageAvailability::Missing.placeholder(),
            Some(PlaceholderKind::Missing)
        );
        assert_eq!(
            ImageAvailability::DecodeFailed.placeholder(),
            Some(PlaceholderKind::Missing)
        );
    }

    #[test]
    fn reference_rasterization_is_deterministic_and_bounded() {
        let a = rasterize_placeholder_reference(PlaceholderKind::Decoding, 37, 5).unwrap();
        let b = rasterize_placeholder_reference(PlaceholderKind::Decoding, 37, 5).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            rasterize_placeholder_reference(PlaceholderKind::Missing, 0, 4),
            Err(PlaceholderError::ZeroSize)
        );
        assert_eq!(
            rasterize_placeholder_reference(PlaceholderKind::Missing, 16_385, 1),
            Err(PlaceholderError::TooLarge)
        );
    }
}

//! 선형 working space와 출력 target encode 공통 수식 (XI.10.1, ADR-0102 결정 1·2).
//!
//! 모든 backend shader와 CPU 기준 rasterizer는 이 모듈의 수식을 따릅니다(아키텍처 §10.3 "내부 색상 연산과 알파
//! 합성은 선형 색공간에서 수행").
//!
//! ## working space
//!
//! - **값**: surface 색공간(협상 결과 [`OutputColorPlan::surface`])의 primaries를 쓰는 **선형광** RGB와 alpha를
//!   곱한 **premultiplied** RGBA `f32`입니다([`WorkingColor`]).
//! - **입력 변환**: 문서·scene 색(비연관 alpha, 인코딩 값, sRGB 또는 Display P3)은
//!   [`OutputColorPlan::convert`](XI.10.5)로 transfer decode → D65 primaries 행렬 → gamut 판정·자르기를 거친 뒤
//!   alpha를 곱합니다([`to_working`]). 같은 변환을 따로 구현하지 않습니다.
//! - **이미지 texel**: decoded 이미지(straight alpha, 비선형 8-bit, XI.7.1)도 같은 경로로 working 값이 됩니다
//!   ([`image_texel`]). 그래서 이미지와 단색 fill은 같은 색 규칙을 따릅니다.
//! - shader는 working 값만 주고받습니다. vertex color·uniform tint·texture sampling 결과·blend 입력이 모두
//!   working 값입니다.
//!
//! ## 출력 encode (target format별)
//!
//! attachment가 저장하는 값은 format으로 정해집니다([`output_encoding`]).
//!
//! | format | [`OutputEncoding`] | 저장 값 |
//! |---|---|---|
//! | `Rgba8UnormSrgb`·`Bgra8UnormSrgb` | `TransferEncoded` | premultiplied 선형 RGB 채널마다 transfer encode(하드웨어), alpha는 선형 그대로 |
//! | `Rgba8Unorm`·`Bgra8Unorm` | `LinearUnorm` | 선형 값 그대로 8-bit(`[0, 1]`로 자름) |
//! | `Rgba16Float` | `LinearFloat` | 선형 값 그대로 half float(자르지 않음) |
//! | `R8Unorm` | `Coverage` | alpha(coverage) 하나 |
//!
//! - transfer encode 곡선은 surface 색공간의 곡선입니다(`ColorSpace::encode_channel`, III.4.2·ADR-0002). sRGB와
//!   Display P3는 같은 곡선이라 하드웨어 sRGB encode format이 두 surface 모두에 맞습니다.
//! - encode는 **premultiplied 채널**에 적용합니다. GPU 하드웨어 sRGB format이 그렇게 저장하고(blend는 decode한
//!   선형 값으로 계산한 뒤 다시 encode), CPU 기준도 같은 순서를 씁니다.
//! - 8-bit 양자화는 `round(v × 255)`(반올림, [`quantize_unorm8`])입니다. GPU 변환의 반올림 방향 차이(최대 1 LSB)는
//!   적합성 허용 오차(XI.11.2)가 흡수합니다.
//! - `Bgra8Unorm`·`Rgba8Unorm` 표시 surface는 선형 값을 저장하므로 backend가 surface 색공간의 **선형** 변형을
//!   붙입니다(Metal: `kCGColorSpaceLinearSRGB`·`kCGColorSpaceLinearDisplayP3`). shader가 따로 encode하지 않습니다.
//!   선형 8-bit는 어두운 영역의 계조가 거칠어지므로 표시 기본값은 `*UnormSrgb`입니다(ADR-0102 결정 2).
//!
//! readback(적합성 harness)은 [`decode_pixel`]로 저장 byte를 working 값으로 되돌립니다.

use crate::output_color::{OutputColor, OutputColorPlan};
use crate::render_graph::{ClearColor, TextureFormat};

pub use stellar_scene::ColorSpace;

/// working space 색: surface primaries 선형광, premultiplied RGBA.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WorkingColor {
    /// premultiplied R, G, B, A.
    pub rgba: [f32; 4],
}

impl WorkingColor {
    /// 투명 검정.
    pub const TRANSPARENT: WorkingColor = WorkingColor { rgba: [0.0; 4] };

    /// premultiplied 값으로 만듭니다.
    pub const fn premultiplied(rgba: [f32; 4]) -> WorkingColor {
        WorkingColor { rgba }
    }

    /// 비연관 선형 RGB와 alpha로 만듭니다(alpha를 곱합니다).
    pub fn from_straight(rgb: [f32; 3], alpha: f32) -> WorkingColor {
        WorkingColor {
            rgba: [rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha],
        }
    }

    /// alpha.
    pub const fn alpha(&self) -> f32 {
        self.rgba[3]
    }

    /// 모든 채널에 `k`를 곱합니다(opacity·coverage).
    pub fn scale(self, k: f32) -> WorkingColor {
        WorkingColor {
            rgba: self.rgba.map(|c| c * k),
        }
    }

    /// 비연관 선형 RGB(alpha 0이면 0).
    pub fn unpremultiplied(&self) -> [f32; 3] {
        let a = self.rgba[3];
        if a <= 0.0 {
            return [0.0; 3];
        }
        [self.rgba[0] / a, self.rgba[1] / a, self.rgba[2] / a]
    }

    /// graph clear 값. 채널이 비유한이면 `None`.
    pub fn to_clear(self) -> Option<ClearColor> {
        use stellar_scene::FiniteF32;
        let [r, g, b, a] = self.rgba;
        Some(ClearColor {
            r: FiniteF32::new(r).ok()?,
            g: FiniteF32::new(g).ok()?,
            b: FiniteF32::new(b).ok()?,
            a: FiniteF32::new(a).ok()?,
        })
    }
}

/// 문서·scene 색을 working 값으로 바꿉니다(모듈 문서 "입력 변환").
pub fn to_working(plan: &OutputColorPlan, color: OutputColor) -> WorkingColor {
    let surface = plan.convert(color);
    WorkingColor::from_straight(surface.linear, surface.alpha)
}

/// decoded 이미지 texel(straight alpha, 비선형 8-bit)을 working 값으로 바꿉니다.
pub fn image_texel(plan: &OutputColorPlan, space: ColorSpace, texel: [u8; 4]) -> WorkingColor {
    let unit = |v: u8| f32::from(v) / 255.0;
    to_working(
        plan,
        OutputColor {
            space,
            rgb: [unit(texel[0]), unit(texel[1]), unit(texel[2])],
            alpha: unit(texel[3]),
        },
    )
}

/// attachment가 값을 저장하는 방식(모듈 문서 표).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputEncoding {
    /// premultiplied 선형 RGB에 transfer encode, alpha 선형(하드웨어 `*UnormSrgb`).
    TransferEncoded,
    /// 선형 값 그대로 8-bit unorm.
    LinearUnorm,
    /// 선형 값 그대로 half float.
    LinearFloat,
    /// alpha(coverage) 한 채널.
    Coverage,
}

/// color format의 저장 방식. depth·stencil format은 `None`.
pub const fn output_encoding(format: TextureFormat) -> Option<OutputEncoding> {
    match format {
        TextureFormat::Rgba8UnormSrgb | TextureFormat::Bgra8UnormSrgb => {
            Some(OutputEncoding::TransferEncoded)
        }
        TextureFormat::Rgba8Unorm | TextureFormat::Bgra8Unorm => Some(OutputEncoding::LinearUnorm),
        TextureFormat::Rgba16Float => Some(OutputEncoding::LinearFloat),
        TextureFormat::R8Unorm => Some(OutputEncoding::Coverage),
        TextureFormat::Depth32FloatStencil8 => None,
    }
}

/// attachment가 저장하는 정규화 값(R, G, B, A). `Coverage`는 `[a, 0, 0, 0]`입니다.
///
/// `surface`는 transfer encode 곡선을 고르는 surface 색공간입니다.
pub fn encode_stored(
    color: WorkingColor,
    encoding: OutputEncoding,
    surface: ColorSpace,
) -> [f32; 4] {
    let unit = |v: f32| v.clamp(0.0, 1.0);
    let [r, g, b, a] = color.rgba;
    match encoding {
        OutputEncoding::TransferEncoded => [
            surface.encode_channel(unit(r)),
            surface.encode_channel(unit(g)),
            surface.encode_channel(unit(b)),
            unit(a),
        ],
        OutputEncoding::LinearUnorm => [unit(r), unit(g), unit(b), unit(a)],
        OutputEncoding::LinearFloat => color.rgba.map(round_to_f16),
        OutputEncoding::Coverage => [unit(a), 0.0, 0.0, 0.0],
    }
}

/// 정규화 값을 8-bit unorm으로 양자화합니다(`round(v × 255)`, `[0, 1]`로 자름).
pub fn quantize_unorm8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u8
}

/// pixel 하나의 byte 수(depth·stencil은 `None`).
pub const fn bytes_per_pixel(format: TextureFormat) -> Option<usize> {
    match format {
        TextureFormat::Rgba8Unorm
        | TextureFormat::Rgba8UnormSrgb
        | TextureFormat::Bgra8Unorm
        | TextureFormat::Bgra8UnormSrgb => Some(4),
        TextureFormat::Rgba16Float => Some(8),
        TextureFormat::R8Unorm => Some(1),
        TextureFormat::Depth32FloatStencil8 => None,
    }
}

const fn is_bgra(format: TextureFormat) -> bool {
    matches!(
        format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
    )
}

/// working 값을 `format` attachment의 저장 byte로 만듭니다(format byte 순서, half float는 little-endian).
pub fn encode_pixel(
    color: WorkingColor,
    format: TextureFormat,
    surface: ColorSpace,
) -> Option<Vec<u8>> {
    let encoding = output_encoding(format)?;
    let stored = encode_stored(color, encoding, surface);
    Some(match encoding {
        OutputEncoding::LinearFloat => stored
            .iter()
            .flat_map(|v| f32_to_f16_bits(*v).to_le_bytes())
            .collect(),
        OutputEncoding::Coverage => vec![quantize_unorm8(stored[0])],
        OutputEncoding::TransferEncoded | OutputEncoding::LinearUnorm => {
            let [r, g, b, a] = stored.map(quantize_unorm8);
            if is_bgra(format) {
                vec![b, g, r, a]
            } else {
                vec![r, g, b, a]
            }
        }
    })
}

/// `format` attachment의 저장 byte(pixel 하나)를 working 값으로 되돌립니다. 길이가 맞지 않으면 `None`.
///
/// `Coverage`는 `[0, 0, 0, coverage]`입니다.
pub fn decode_pixel(
    bytes: &[u8],
    format: TextureFormat,
    surface: ColorSpace,
) -> Option<WorkingColor> {
    let encoding = output_encoding(format)?;
    if bytes.len() != bytes_per_pixel(format)? {
        return None;
    }
    let unit = |v: u8| f32::from(v) / 255.0;
    Some(WorkingColor::premultiplied(match encoding {
        OutputEncoding::LinearFloat => {
            let half = |i: usize| f16_bits_to_f32(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
            [half(0), half(2), half(4), half(6)]
        }
        OutputEncoding::Coverage => [0.0, 0.0, 0.0, unit(bytes[0])],
        OutputEncoding::TransferEncoded | OutputEncoding::LinearUnorm => {
            let (r, g, b, a) = if is_bgra(format) {
                (bytes[2], bytes[1], bytes[0], bytes[3])
            } else {
                (bytes[0], bytes[1], bytes[2], bytes[3])
            };
            let decode = |v: u8| match encoding {
                OutputEncoding::TransferEncoded => surface.decode_channel(unit(v)),
                _ => unit(v),
            };
            [decode(r), decode(g), decode(b), unit(a)]
        }
    }))
}

/// `f32`를 가장 가까운 IEEE 754 half 값으로 반올림한 `f32`(half 저장 결과).
pub fn round_to_f16(value: f32) -> f32 {
    f16_bits_to_f32(f32_to_f16_bits(value))
}

/// `f32` → IEEE 754 binary16 bits(round-to-nearest-even, 범위 밖은 ±무한대, NaN 보존).
pub fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x007F_FFFF;
    if exponent == 0xFF {
        // 무한대·NaN.
        let nan = if mantissa != 0 { 0x0200 } else { 0 };
        return sign | 0x7C00 | nan;
    }
    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1F {
        return sign | 0x7C00;
    }
    if half_exponent <= 0 {
        // subnormal 또는 0.
        if half_exponent < -10 {
            return sign;
        }
        let full = mantissa | 0x0080_0000;
        let shift = (14 - half_exponent) as u32;
        let mut half = full >> shift;
        let remainder = full & ((1 << shift) - 1);
        let halfway = 1 << (shift - 1);
        if remainder > halfway || (remainder == halfway && (half & 1) == 1) {
            half += 1;
        }
        return sign | half as u16;
    }
    let mut half = ((half_exponent as u32) << 10) | (mantissa >> 13);
    let remainder = mantissa & 0x1FFF;
    if remainder > 0x1000 || (remainder == 0x1000 && (half & 1) == 1) {
        // 올림이 지수로 넘어가도 bits 덧셈이 맞는 값을 만듭니다(최대값은 무한대).
        half += 1;
    }
    sign | half as u16
}

/// IEEE 754 binary16 bits → `f32`.
pub fn f16_bits_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = u32::from((bits >> 10) & 0x1F);
    let mantissa = u32::from(bits & 0x03FF);
    let out = match (exponent, mantissa) {
        (0, 0) => sign,
        (0, m) => {
            // subnormal: m × 2^-24.
            let value = m as f32 * f32::powi(2.0, -24);
            return if sign != 0 { -value } else { value };
        }
        (0x1F, 0) => sign | 0x7F80_0000,
        (0x1F, m) => sign | 0x7F80_0000 | (m << 13),
        (e, m) => sign | ((e + 127 - 15) << 23) | (m << 13),
    };
    f32::from_bits(out)
}

#[cfg(test)]
mod tests;

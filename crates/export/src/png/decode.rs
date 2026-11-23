//! PNG 해석 (XX.2.4 golden 비교, 이미지 asset pixel).
//!
//! 신뢰하지 않는 입력을 가정합니다. chunk CRC를 검사하고, 크기·해제 상한([`DecodeLimits`])을 넘거나 잘린·잘못된
//! 입력이면 [`PngDecodeError`]이며 panic하지 않습니다.
//!
//! - 지원: 비interlace, 색 형식 0(gray)·2(RGB)·3(palette)·4(gray+α)·6(RGBA), bit depth 1·2·4·8·16, `tRNS` 투명색.
//!   16-bit 표본은 상위 8 bit로 줄입니다(내보내기 합성은 8-bit 출력).
//! - Adam7 interlace는 [`PngDecodeError::Unsupported`]입니다.
//! - 색 정보 chunk(`sRGB`·`cICP`·`iCCP`·`gAMA`·`pHYs`)는 [`PngColorInfo`]로 돌려주고, [`PngColorInfo::color_space`]가
//!   sRGB·Display P3 판정 규칙을 정합니다.

use std::fmt;

use stellar_core_types::color::ColorSpace;

use super::icc;
use super::zlib::{crc32, decompress, InflateError};

/// PNG signature.
pub const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// 해석 상한.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    /// 최대 pixel 수.
    pub max_pixels: u64,
    /// 해제한 IDAT·iCCP 최대 byte.
    pub max_inflated: usize,
}

impl Default for DecodeLimits {
    fn default() -> DecodeLimits {
        DecodeLimits {
            max_pixels: 1 << 26,
            max_inflated: 1 << 30,
        }
    }
}

/// 해석 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngDecodeError {
    /// PNG signature가 아닙니다.
    NotPng,
    /// 입력이 중간에 끝났습니다.
    Truncated,
    /// chunk CRC가 맞지 않습니다.
    Crc {
        /// chunk 형식.
        chunk: [u8; 4],
    },
    /// chunk 순서·내용이 형식에 어긋납니다.
    Malformed {
        /// 설명.
        reason: &'static str,
    },
    /// 지원하지 않는 변형입니다.
    Unsupported {
        /// 설명.
        reason: &'static str,
    },
    /// 크기·해제 상한을 넘습니다.
    TooLarge,
    /// 압축 data 오류.
    Inflate(InflateError),
}

impl fmt::Display for PngDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PngDecodeError::NotPng => f.write_str("PNG signature가 아닙니다"),
            PngDecodeError::Truncated => f.write_str("PNG data가 중간에 끝났습니다"),
            PngDecodeError::Crc { chunk } => {
                write!(
                    f,
                    "{} chunk CRC가 맞지 않습니다",
                    String::from_utf8_lossy(chunk)
                )
            }
            PngDecodeError::Malformed { reason } => write!(f, "PNG 형식 오류: {reason}"),
            PngDecodeError::Unsupported { reason } => write!(f, "지원하지 않는 PNG: {reason}"),
            PngDecodeError::TooLarge => f.write_str("PNG가 크기 상한을 넘습니다"),
            PngDecodeError::Inflate(error) => write!(f, "PNG 압축 data 오류: {error}"),
        }
    }
}

impl std::error::Error for PngDecodeError {}

impl From<InflateError> for PngDecodeError {
    fn from(error: InflateError) -> PngDecodeError {
        match error {
            InflateError::TooLarge => PngDecodeError::TooLarge,
            other => PngDecodeError::Inflate(other),
        }
    }
}

/// `cICP` chunk 값(ITU-T H.273 code point).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cicp {
    /// colour primaries(1 = BT.709/sRGB, 12 = SMPTE EG 432-1 Display P3).
    pub primaries: u8,
    /// transfer characteristics(13 = sRGB 곡선).
    pub transfer: u8,
    /// matrix coefficients(RGB는 0).
    pub matrix: u8,
    /// full range 표시(1).
    pub full_range: u8,
}

/// 색 정보 chunk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PngColorInfo {
    /// `sRGB` rendering intent.
    pub srgb_intent: Option<u8>,
    /// `cICP`.
    pub cicp: Option<Cicp>,
    /// `iCCP` profile 이름과 해제한 profile.
    pub icc: Option<(String, Vec<u8>)>,
    /// `gAMA`(× 100000).
    pub gamma: Option<u32>,
    /// `pHYs`(x, y pixel per unit, unit).
    pub physical: Option<(u32, u32, u8)>,
}

/// 색 정보가 sRGB·Display P3가 아닙니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedColorInfo;

impl PngColorInfo {
    /// 이미지 pixel의 색공간.
    ///
    /// `cICP`가 있으면 그것이 우선합니다(PNG 3판): primaries 1 → sRGB, 12 → Display P3, transfer는 13(sRGB 곡선)이어야
    /// 합니다. 없으면 `iCCP` colorant([`icc::primaries_space`]), 그것도 없으면 sRGB입니다(`sRGB` chunk, 또는 색 정보
    /// 없는 PNG의 관례). 그 밖은 [`UnsupportedColorInfo`]입니다.
    pub fn color_space(&self) -> Result<ColorSpace, UnsupportedColorInfo> {
        if let Some(cicp) = self.cicp {
            if cicp.transfer != 13 || cicp.matrix != 0 {
                return Err(UnsupportedColorInfo);
            }
            return match cicp.primaries {
                1 => Ok(ColorSpace::Srgb),
                12 => Ok(ColorSpace::DisplayP3),
                _ => Err(UnsupportedColorInfo),
            };
        }
        if let Some((_, profile)) = &self.icc {
            return icc::primaries_space(profile).ok_or(UnsupportedColorInfo);
        }
        Ok(ColorSpace::Srgb)
    }
}

/// 해석한 PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPng {
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
    /// 원본 bit depth.
    pub bit_depth: u8,
    /// 원본 색 형식.
    pub color_type: u8,
    /// RGBA8 pixel(비연관 알파, 행 우선).
    pub rgba: Vec<u8>,
    /// 색 정보.
    pub color: PngColorInfo,
    /// chunk 형식 순서(검사용).
    pub chunks: Vec<[u8; 4]>,
}

impl DecodedPng {
    /// pixel `(x, y)`의 RGBA.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = ((y as usize) * (self.width as usize) + x as usize) * 4;
        self.rgba.get(at..at + 4)?.try_into().ok()
    }
}

fn be32(bytes: &[u8], at: usize) -> Result<u32, PngDecodeError> {
    let raw: [u8; 4] = bytes
        .get(at..at + 4)
        .ok_or(PngDecodeError::Truncated)?
        .try_into()
        .map_err(|_| PngDecodeError::Truncated)?;
    Ok(u32::from_be_bytes(raw))
}

/// Paeth 예측(PNG filter 4).
pub(crate) fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// filter 한 행을 되돌립니다. `bpp`는 filter 단위 byte 수(최소 1)입니다.
pub(crate) fn unfilter_row(
    kind: u8,
    row: &mut [u8],
    previous: &[u8],
    bpp: usize,
) -> Result<(), PngDecodeError> {
    match kind {
        0 => {}
        1 => {
            for i in bpp..row.len() {
                row[i] = row[i].wrapping_add(row[i - bpp]);
            }
        }
        2 => {
            for i in 0..row.len() {
                row[i] = row[i].wrapping_add(previous[i]);
            }
        }
        3 => {
            for i in 0..row.len() {
                let left = if i >= bpp { row[i - bpp] } else { 0 };
                let avg = ((u16::from(left) + u16::from(previous[i])) / 2) as u8;
                row[i] = row[i].wrapping_add(avg);
            }
        }
        4 => {
            for i in 0..row.len() {
                let left = if i >= bpp { row[i - bpp] } else { 0 };
                let up_left = if i >= bpp { previous[i - bpp] } else { 0 };
                row[i] = row[i].wrapping_add(paeth(left, previous[i], up_left));
            }
        }
        _ => {
            return Err(PngDecodeError::Malformed {
                reason: "알 수 없는 filter 형식",
            })
        }
    }
    Ok(())
}

/// PNG를 해석합니다(모듈 문서).
pub fn decode_png(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedPng, PngDecodeError> {
    if bytes.get(..8) != Some(&SIGNATURE[..]) {
        return Err(PngDecodeError::NotPng);
    }
    let mut at = 8usize;
    let mut header: Option<(u32, u32, u8, u8)> = None;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut transparency: Vec<u8> = Vec::new();
    let mut idat: Vec<u8> = Vec::new();
    let mut color = PngColorInfo::default();
    let mut chunks = Vec::new();
    let mut ended = false;
    while !ended {
        let length = be32(bytes, at)? as usize;
        let kind: [u8; 4] = bytes
            .get(at + 4..at + 8)
            .ok_or(PngDecodeError::Truncated)?
            .try_into()
            .map_err(|_| PngDecodeError::Truncated)?;
        let data_end = at
            .checked_add(8)
            .and_then(|v| v.checked_add(length))
            .ok_or(PngDecodeError::Truncated)?;
        let data = bytes
            .get(at + 8..data_end)
            .ok_or(PngDecodeError::Truncated)?;
        let crc = be32(bytes, data_end)?;
        if crc32(&bytes[at + 4..data_end]) != crc {
            return Err(PngDecodeError::Crc { chunk: kind });
        }
        at = data_end + 4;
        chunks.push(kind);
        if chunks.len() == 1 && &kind != b"IHDR" {
            return Err(PngDecodeError::Malformed {
                reason: "첫 chunk가 IHDR이 아닙니다",
            });
        }
        match &kind {
            b"IHDR" => {
                if header.is_some() || data.len() != 13 {
                    return Err(PngDecodeError::Malformed {
                        reason: "IHDR가 잘못되었습니다",
                    });
                }
                let (width, height) = (be32(data, 0)?, be32(data, 4)?);
                let (depth, color_type) = (data[8], data[9]);
                if data[10] != 0 || data[11] != 0 {
                    return Err(PngDecodeError::Malformed {
                        reason: "압축·filter 방식이 0이 아닙니다",
                    });
                }
                if data[12] != 0 {
                    return Err(PngDecodeError::Unsupported {
                        reason: "interlace PNG",
                    });
                }
                let valid = match color_type {
                    0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
                    3 => matches!(depth, 1 | 2 | 4 | 8),
                    2 | 4 | 6 => matches!(depth, 8 | 16),
                    _ => false,
                };
                if !valid || width == 0 || height == 0 {
                    return Err(PngDecodeError::Malformed {
                        reason: "색 형식·bit depth·크기가 잘못되었습니다",
                    });
                }
                if u64::from(width) * u64::from(height) > limits.max_pixels {
                    return Err(PngDecodeError::TooLarge);
                }
                header = Some((width, height, depth, color_type));
            }
            b"PLTE" => {
                if data.len() % 3 != 0 || data.len() > 768 {
                    return Err(PngDecodeError::Malformed {
                        reason: "PLTE 길이가 잘못되었습니다",
                    });
                }
                palette = data.chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
            }
            b"tRNS" => transparency = data.to_vec(),
            b"IDAT" => {
                if idat.len() + data.len() > limits.max_inflated {
                    return Err(PngDecodeError::TooLarge);
                }
                idat.extend_from_slice(data);
            }
            b"sRGB" => color.srgb_intent = data.first().copied(),
            b"gAMA" => color.gamma = Some(be32(data, 0)?),
            b"pHYs" => {
                if data.len() == 9 {
                    color.physical = Some((be32(data, 0)?, be32(data, 4)?, data[8]));
                }
            }
            b"cICP" => {
                if data.len() == 4 {
                    color.cicp = Some(Cicp {
                        primaries: data[0],
                        transfer: data[1],
                        matrix: data[2],
                        full_range: data[3],
                    });
                }
            }
            b"iCCP" => {
                let name_end =
                    data.iter()
                        .position(|b| *b == 0)
                        .ok_or(PngDecodeError::Malformed {
                            reason: "iCCP 이름이 끝나지 않습니다",
                        })?;
                if data.get(name_end + 1) != Some(&0) {
                    return Err(PngDecodeError::Malformed {
                        reason: "iCCP 압축 방식이 0이 아닙니다",
                    });
                }
                let name = String::from_utf8_lossy(&data[..name_end]).into_owned();
                let profile = decompress(&data[name_end + 2..], limits.max_inflated.min(1 << 24))?;
                color.icc = Some((name, profile));
            }
            b"IEND" => ended = true,
            _ => {
                // 필수(대문자 시작) chunk를 모르면 해석할 수 없습니다.
                if kind[0].is_ascii_uppercase() {
                    return Err(PngDecodeError::Unsupported {
                        reason: "알 수 없는 필수 chunk",
                    });
                }
            }
        }
    }
    let (width, height, depth, color_type) = header.ok_or(PngDecodeError::Malformed {
        reason: "IHDR가 없습니다",
    })?;
    if color_type == 3 && palette.is_empty() {
        return Err(PngDecodeError::Malformed {
            reason: "palette PNG에 PLTE가 없습니다",
        });
    }
    let channels = match color_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        _ => 4,
    };
    let bits_per_pixel = channels * usize::from(depth);
    let stride = (width as usize * bits_per_pixel).div_ceil(8);
    let bpp = bits_per_pixel.div_ceil(8).max(1);
    let expected = (stride + 1)
        .checked_mul(height as usize)
        .ok_or(PngDecodeError::TooLarge)?;
    if expected > limits.max_inflated {
        return Err(PngDecodeError::TooLarge);
    }
    let raw = decompress(&idat, expected)?;
    if raw.len() != expected {
        return Err(PngDecodeError::Malformed {
            reason: "IDAT 길이가 크기와 맞지 않습니다",
        });
    }
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    let mut previous = vec![0u8; stride];
    let mut row = vec![0u8; stride];
    let sample = |row: &[u8], index: usize| -> u16 {
        match depth {
            16 => u16::from_be_bytes([row[index * 2], row[index * 2 + 1]]),
            8 => u16::from(row[index]),
            _ => {
                let per_byte = 8 / usize::from(depth);
                let byte = row[index / per_byte];
                let shift = 8 - usize::from(depth) * (index % per_byte + 1);
                u16::from((byte >> shift) & ((1u8 << depth) - 1))
            }
        }
    };
    let to8 = |value: u16| -> u8 {
        match depth {
            16 => (value >> 8) as u8,
            8 => value as u8,
            _ => (u32::from(value) * 255 / ((1u32 << depth) - 1)) as u8,
        }
    };
    let gray_key = (color_type == 0 && transparency.len() >= 2)
        .then(|| u16::from_be_bytes([transparency[0], transparency[1]]));
    let rgb_key = (color_type == 2 && transparency.len() >= 6).then(|| {
        [
            u16::from_be_bytes([transparency[0], transparency[1]]),
            u16::from_be_bytes([transparency[2], transparency[3]]),
            u16::from_be_bytes([transparency[4], transparency[5]]),
        ]
    });
    for y in 0..height as usize {
        let line = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        row.copy_from_slice(&line[1..]);
        unfilter_row(line[0], &mut row, &previous, bpp)?;
        for x in 0..width as usize {
            let pixel = match color_type {
                0 => {
                    let v = sample(&row, x);
                    let alpha = if gray_key == Some(v) { 0 } else { 255 };
                    let g = to8(v);
                    [g, g, g, alpha]
                }
                2 => {
                    let v = [
                        sample(&row, x * 3),
                        sample(&row, x * 3 + 1),
                        sample(&row, x * 3 + 2),
                    ];
                    let alpha = if rgb_key == Some(v) { 0 } else { 255 };
                    [to8(v[0]), to8(v[1]), to8(v[2]), alpha]
                }
                3 => {
                    let index = usize::from(sample(&row, x));
                    let rgb = palette.get(index).ok_or(PngDecodeError::Malformed {
                        reason: "palette 범위 밖 index",
                    })?;
                    let alpha = transparency.get(index).copied().unwrap_or(255);
                    [rgb[0], rgb[1], rgb[2], alpha]
                }
                4 => {
                    let g = to8(sample(&row, x * 2));
                    [g, g, g, to8(sample(&row, x * 2 + 1))]
                }
                _ => [
                    to8(sample(&row, x * 4)),
                    to8(sample(&row, x * 4 + 1)),
                    to8(sample(&row, x * 4 + 2)),
                    to8(sample(&row, x * 4 + 3)),
                ],
            };
            rgba.extend_from_slice(&pixel);
        }
        std::mem::swap(&mut previous, &mut row);
    }
    Ok(DecodedPng {
        width,
        height,
        bit_depth: depth,
        color_type,
        rgba,
        color,
        chunks,
    })
}

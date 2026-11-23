//! RGBA image → PNG bytes (XX.2.2, XX.2.3).
//!
//! - IHDR: 8-bit RGBA(색 형식 6), 비interlace.
//! - 색 정보(XX.2.2):
//!   - sRGB: `sRGB`(perceptual) + `gAMA`(45455) + `cHRM`(sRGB primaries). PNG 명세가 권하는 호환 조합입니다.
//!   - Display P3: `cICP`(primaries 12, transfer 13, matrix 0, full range) + `iCCP`("Display P3", 생성 ICC v4 profile,
//!     [`super::icc::display_p3_profile`]). `cICP`를 읽는 해석기는 그것을, 아니면 ICC profile을 씁니다.
//! - `pHYs`: 배율 1을 72 dpi로 보고 `배율 × 72 / 0.0254` pixel/m를 기록합니다(배율 2 = 144 dpi = @2x 관례).
//! - filter: 행마다 다섯 filter 중 filter 결과의 부호 있는 byte 절댓값 합이 가장 작은 것(같으면 번호가 작은 것).
//! - IDAT: [`super::zlib::compress`] 결과를 [`IDAT_CHUNK`] byte씩 나눕니다.
//!
//! 시각(`tIME`)·텍스트 chunk를 쓰지 않으므로 같은 image·배율이면 bytes가 같습니다.

use super::decode::paeth;
use super::icc::display_p3_profile;
use super::zlib::{compress, crc32_update};
use super::SIGNATURE;
use crate::raster::{CancelToken, OutputProfile, RgbaImage};

/// IDAT chunk 하나의 최대 data byte.
pub const IDAT_CHUNK: usize = 1 << 20;

/// encode가 취소되었습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodeCancelled;

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32_update(crc32_update(0xFFFF_FFFF, kind), data) ^ 0xFFFF_FFFF;
    out.extend_from_slice(&crc.to_be_bytes());
}

/// 행 하나를 `kind` filter로 바꿉니다(bpp 4).
fn filter_row(kind: u8, row: &[u8], previous: &[u8], out: &mut Vec<u8>) {
    const BPP: usize = 4;
    out.clear();
    for i in 0..row.len() {
        let left = if i >= BPP { row[i - BPP] } else { 0 };
        let up = previous[i];
        let up_left = if i >= BPP { previous[i - BPP] } else { 0 };
        let predicted = match kind {
            0 => 0,
            1 => left,
            2 => up,
            3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
            _ => paeth(left, up, up_left),
        };
        out.push(row[i].wrapping_sub(predicted));
    }
}

fn cost(filtered: &[u8]) -> u64 {
    filtered
        .iter()
        .map(|b| u64::from((*b as i8).unsigned_abs()))
        .sum()
}

/// 배율의 `pHYs` pixel/m.
pub fn pixels_per_meter(scale: f32) -> u32 {
    let value = (f64::from(scale) * 72.0 / 0.0254).round();
    if value.is_finite() && value >= 1.0 {
        value.min(f64::from(u32::MAX)) as u32
    } else {
        1
    }
}

/// image를 PNG로 encode합니다(모듈 문서). `scale`은 `pHYs`에 씁니다. `cancel`은 행마다 확인합니다.
pub fn encode_png(
    image: &RgbaImage,
    scale: f32,
    cancel: &CancelToken,
) -> Result<Vec<u8>, EncodeCancelled> {
    let width = image.width() as usize;
    let stride = width * 4;
    let mut raw = Vec::with_capacity((stride + 1) * image.height() as usize);
    let zero = vec![0u8; stride];
    let mut candidate = Vec::with_capacity(stride);
    let mut best = Vec::with_capacity(stride);
    for (y, row) in image.pixels().chunks(stride).enumerate() {
        if cancel.is_cancelled() {
            return Err(EncodeCancelled);
        }
        let previous = if y == 0 {
            &zero[..]
        } else {
            &image.pixels()[(y - 1) * stride..y * stride]
        };
        let mut best_kind = 0u8;
        let mut best_cost = u64::MAX;
        for kind in 0..5u8 {
            filter_row(kind, row, previous, &mut candidate);
            let c = cost(&candidate);
            if c < best_cost {
                best_cost = c;
                best_kind = kind;
                std::mem::swap(&mut best, &mut candidate);
            }
        }
        raw.push(best_kind);
        raw.extend_from_slice(&best);
        cancel.advance(1);
    }
    let packed = compress(&raw, &mut || !cancel.is_cancelled()).map_err(|_| EncodeCancelled)?;
    let mut out = Vec::with_capacity(packed.len() + 4096);
    out.extend_from_slice(&SIGNATURE);
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&image.width().to_be_bytes());
    header.extend_from_slice(&image.height().to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    match image.profile() {
        OutputProfile::Srgb => {
            chunk(&mut out, b"sRGB", &[0]);
            chunk(&mut out, b"gAMA", &45_455u32.to_be_bytes());
            let mut chrm = Vec::with_capacity(32);
            for v in [
                31_270u32, 32_900, 64_000, 33_000, 30_000, 60_000, 15_000, 6_000,
            ] {
                chrm.extend_from_slice(&v.to_be_bytes());
            }
            chunk(&mut out, b"cHRM", &chrm);
        }
        OutputProfile::DisplayP3 => {
            chunk(&mut out, b"cICP", &[12, 13, 0, 1]);
            let profile =
                compress(&display_p3_profile(), &mut || true).map_err(|_| EncodeCancelled)?;
            let mut iccp = b"Display P3\0\0".to_vec();
            iccp.extend_from_slice(&profile);
            chunk(&mut out, b"iCCP", &iccp);
        }
    }
    let ppm = pixels_per_meter(scale);
    let mut phys = Vec::with_capacity(9);
    phys.extend_from_slice(&ppm.to_be_bytes());
    phys.extend_from_slice(&ppm.to_be_bytes());
    phys.push(1);
    chunk(&mut out, b"pHYs", &phys);
    for part in packed.chunks(IDAT_CHUNK) {
        chunk(&mut out, b"IDAT", part);
    }
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

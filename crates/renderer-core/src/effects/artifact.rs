//! reference image와 raw RGBA·PNG artifact (XI.11.1, ADR-0102 결정 6).
//!
//! - [`RgbaImage`]: target이 저장한 8-bit RGBA byte(행 우선, 행 padding 없음). sRGB target이면 transfer encode한
//!   premultiplied 값, 선형 target이면 선형 premultiplied 값입니다([`crate::color`]).
//! - **raw RGBA**(`.rgba`): magic `STRGBA1\0`(8 byte) ++ 너비 u32 LE ++ 높이 u32 LE ++ pixel byte. 비교 기준으로 정확히
//!   되읽습니다([`RgbaImage::from_raw`]).
//! - **PNG**(`.png`): 사람이 볼 사본. 8-bit RGBA(color type 6), 무압축 deflate block. 저장 byte를 그대로 넣으므로
//!   반투명 pixel은 premultiplied 값이 보입니다.
//!
//! 제품 export 기능이 아닙니다(계획 XI.11.1).

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// raw RGBA magic.
pub const RAW_MAGIC: &[u8; 8] = b"STRGBA1\0";

/// 8-bit RGBA image.
#[derive(Clone, PartialEq, Eq)]
pub struct RgbaImage {
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
    /// RGBA byte.
    pub pixels: Vec<u8>,
}

impl fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RgbaImage({}x{}, fnv64 {:016x})",
            self.width,
            self.height,
            crate::digest::fnv1a64(&self.pixels)
        )
    }
}

/// raw 해석 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawError {
    /// magic이 다름.
    BadMagic,
    /// 길이가 크기와 맞지 않음.
    Length,
}

impl RgbaImage {
    /// pixel 하나.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let o = ((y * self.width + x) * 4) as usize;
        [
            self.pixels[o],
            self.pixels[o + 1],
            self.pixels[o + 2],
            self.pixels[o + 3],
        ]
    }

    /// 크기·byte 수가 맞으면 참.
    pub fn is_well_formed(&self) -> bool {
        self.pixels.len() == self.width as usize * self.height as usize * 4
    }

    /// raw RGBA byte.
    pub fn to_raw(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.pixels.len());
        out.extend_from_slice(RAW_MAGIC);
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.pixels);
        out
    }

    /// [`RgbaImage::to_raw`]의 역.
    pub fn from_raw(bytes: &[u8]) -> Result<RgbaImage, RawError> {
        if bytes.len() < 16 || &bytes[..8] != RAW_MAGIC {
            return Err(RawError::BadMagic);
        }
        let width = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let height = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let image = RgbaImage {
            width,
            height,
            pixels: bytes[16..].to_vec(),
        };
        if !image.is_well_formed() {
            return Err(RawError::Length);
        }
        Ok(image)
    }

    /// PNG byte.
    pub fn to_png(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity(self.pixels.len() + self.height as usize);
        let row = self.width as usize * 4;
        for y in 0..self.height as usize {
            raw.push(0); // filter: None
            raw.extend_from_slice(&self.pixels[y * row..(y + 1) * row]);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend_from_slice(&self.width.to_be_bytes());
        ihdr.extend_from_slice(&self.height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &zlib_stored(&raw));
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// `dir/<name>.rgba`와 `dir/<name>.png`를 씁니다. 쓴 raw 경로를 돌려줍니다.
    pub fn write_reference(&self, dir: &Path, name: &str) -> io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let raw = dir.join(format!("{name}.rgba"));
        std::fs::write(&raw, self.to_raw())?;
        std::fs::write(dir.join(format!("{name}.png")), self.to_png())?;
        Ok(raw)
    }
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// zlib stream(무압축 deflate block).
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(65_535).peekable();
    if blocks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(block) = blocks.next() {
        out.push(u8::from(blocks.peek().is_none()));
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> RgbaImage {
        RgbaImage {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 128],
        }
    }

    #[test]
    fn raw_round_trips_and_rejects_bad_input() {
        let img = image();
        let raw = img.to_raw();
        assert_eq!(&raw[..8], RAW_MAGIC);
        assert_eq!(RgbaImage::from_raw(&raw), Ok(img));
        assert_eq!(RgbaImage::from_raw(b"nope"), Err(RawError::BadMagic));
        assert_eq!(
            RgbaImage::from_raw(&raw[..raw.len() - 1]),
            Err(RawError::Length)
        );
    }

    #[test]
    fn png_has_valid_structure_and_checksums() {
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        let png = image().to_png();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        // IDAT: 2 byte zlib header + block header 5 + filter byte + 8 pixel byte + adler 4.
        let idat_len = u32::from_be_bytes([png[33], png[34], png[35], png[36]]);
        assert_eq!(idat_len, 2 + 5 + 1 + 8 + 4);
    }
}

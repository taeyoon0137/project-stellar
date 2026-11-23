//! PNG 내보내기 (XX.2, ADR-0103).
//!
//! 흐름: 진행 허가([`ConfirmedExport`](crate::preflight::ConfirmedExport)) → offscreen target 렌더
//! ([`crate::raster`]) → PNG encode([`encode::encode_png`]) → 저장. 렌더와 encode는 worker thread([`job::PngJob`])에서
//! 돌고 취소할 수 있습니다. 저장(temp + atomic rename)은 파일 시스템 정책을 가진 host가 맡습니다(`stellar-ops`
//! `export_png`, storage `write_file_atomic`).
//!
//! ## encoder 지원 표 (XX.2.2)
//!
//! | 출력 profile | bit depth 8 | bit depth 16 |
//! |---|---|---|
//! | sRGB | 지원(`sRGB`·`gAMA`·`cHRM`) | 미지원 → [`PngOptionsError::UnsupportedBitDepth`] |
//! | Display P3 | 지원(`cICP`·`iCCP`) | 미지원 → [`PngOptionsError::UnsupportedBitDepth`] |
//!
//! 그 밖의 profile 이름(예: `rec2020`, `adobe-rgb`, HDR)은 [`PngOptionsError::UnsupportedProfile`]입니다. 조용히 sRGB로
//! 바꾸지 않습니다.

use std::fmt;

use crate::raster::{OutputProfile, UnknownProfile};

/// zlib·DEFLATE와 CRC (XX.2.3).
pub mod zlib;

/// ICC profile 생성과 판정 (XX.2.2).
pub mod icc;

/// PNG 해석 (XX.2.4).
pub mod decode;

/// PNG encode (XX.2.2, XX.2.3).
pub mod encode;

pub use decode::{decode_png, Cicp, DecodeLimits, DecodedPng, PngColorInfo, PngDecodeError};
pub use encode::encode_png;

/// PNG signature.
pub const SIGNATURE: [u8; 8] = decode::SIGNATURE;

/// PNG 표본 bit depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PngBitDepth {
    /// 8-bit(지원).
    #[default]
    Eight,
    /// 16-bit(미지원, 지원 표).
    Sixteen,
}

impl PngBitDepth {
    /// bit 수.
    pub const fn bits(self) -> u8 {
        match self {
            PngBitDepth::Eight => 8,
            PngBitDepth::Sixteen => 16,
        }
    }
}

/// PNG encoder 옵션 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngOptionsError {
    /// 지원하지 않는 출력 profile.
    UnsupportedProfile(UnknownProfile),
    /// 지원하지 않는 bit depth.
    UnsupportedBitDepth {
        /// 요청 bit 수.
        bits: u8,
    },
}

impl fmt::Display for PngOptionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PngOptionsError::UnsupportedProfile(error) => write!(f, "{error}"),
            PngOptionsError::UnsupportedBitDepth { bits } => {
                write!(
                    f,
                    "PNG encoder는 {bits}-bit 출력을 지원하지 않습니다(8-bit만 지원)"
                )
            }
        }
    }
}

impl std::error::Error for PngOptionsError {}

/// 검증된 PNG encoder 옵션(지원 표 안의 조합만).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PngOptions {
    profile: OutputProfile,
}

impl PngOptions {
    /// profile과 bit depth 조합을 검사해 옵션을 만듭니다.
    pub fn new(
        profile: OutputProfile,
        bit_depth: PngBitDepth,
    ) -> Result<PngOptions, PngOptionsError> {
        match bit_depth {
            PngBitDepth::Eight => Ok(PngOptions { profile }),
            PngBitDepth::Sixteen => Err(PngOptionsError::UnsupportedBitDepth { bits: 16 }),
        }
    }

    /// 이름(`srgb`·`display-p3`)과 bit 수(8·16)로 만듭니다. CLI·agent 입력 경계용입니다.
    pub fn from_names(profile: &str, bits: u8) -> Result<PngOptions, PngOptionsError> {
        let profile =
            OutputProfile::from_name(profile).map_err(PngOptionsError::UnsupportedProfile)?;
        let depth = match bits {
            8 => PngBitDepth::Eight,
            16 => PngBitDepth::Sixteen,
            bits => return Err(PngOptionsError::UnsupportedBitDepth { bits }),
        };
        PngOptions::new(profile, depth)
    }

    /// 출력 profile.
    pub const fn profile(&self) -> OutputProfile {
        self.profile
    }

    /// bit depth(항상 8).
    pub const fn bit_depth(&self) -> PngBitDepth {
        PngBitDepth::Eight
    }
}

#[cfg(test)]
mod tests;

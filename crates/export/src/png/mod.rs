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

/// zlib·DEFLATE와 CRC (XX.2.3).
pub mod zlib;

/// ICC profile 생성과 판정 (XX.2.2).
pub mod icc;

/// PNG 해석 (XX.2.4).
pub mod decode;

pub use decode::{decode_png, Cicp, DecodeLimits, DecodedPng, PngColorInfo, PngDecodeError};

/// PNG signature.
pub const SIGNATURE: [u8; 8] = decode::SIGNATURE;

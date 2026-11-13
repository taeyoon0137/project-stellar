//! `AssetId` — StableId newtype 에셋 식별자 (XIII.5.1).
//!
//! 에셋의 **논리** 식별자입니다. 같은 content hash를 여러 AssetId가 참조할 수 있으므로
//! (XIII.5.3 dedup) 내용 주소와 별개인 StableId 도메인으로 둡니다. Node·chunk 등 다른
//! StableId 도메인과 섞이지 않도록 core-types [`StableId`]를 newtype으로 감쌉니다
//! (crate 의존 정책상 `assets → core-types` 직접 의존 허용, 2026-11-14).
//!
//! # 표현
//!
//! 내부 값이 [`StableId`]이므로 bytes·정렬·문자열·파싱 오류가 모두 StableId 그대로입니다.
//! - binary·canonical: 16바이트(big-endian, UUIDv7 레이아웃) raw. 이전 표현(`[u8; 16]`)과
//!   **같은 bytes**이므로 기존 인코딩 fixture가 그대로 유효합니다.
//! - 정렬: StableId와 같은 byte 사전식(= UUIDv7 timestamp 순).
//! - 문자열: 표준 소문자 hyphen UUID(StableId `Display`/`FromStr`).
//! - StableId와의 변환: [`AssetId::from_stable_id`]·[`AssetId::stable_id`]와 `From` 구현.

use std::fmt;
use std::str::FromStr;

use stellar_core_types::ids::{ParseStableIdError, StableId};
use stellar_storage::canonical::decode::{CanonicalDecoder, DecodeError};
use stellar_storage::canonical::encode::CanonicalEncoder;

/// `AssetId` 문자열 파싱 오류(StableId 문자열 규칙 그대로).
pub type ParseAssetIdError = ParseStableIdError;

/// StableId newtype 에셋 식별자.
///
/// StableId와는 명시 변환으로만 오갑니다.
///
/// ```
/// use stellar_assets::id::AssetId;
/// use stellar_core_types::ids::StableId;
/// let stable = StableId::new_v7(0x0192_3c4d_5e6f, 0xabc, 0x0123_4567_89ab_cdef);
/// let asset = AssetId::from_stable_id(stable);
/// assert_eq!(asset.stable_id(), stable);
/// assert_eq!(asset.to_string(), stable.to_string());
/// ```
///
/// # 다른 StableId 도메인과 혼용은 컴파일 실패
///
/// `AssetId`를 요구하는 자리에 다른 도메인 식별자(예: [`ChunkId`](stellar_storage::chunk::ChunkId))나
/// 감싸지 않은 [`StableId`]를 넘길 수 없습니다.
///
/// ```compile_fail
/// use stellar_assets::id::AssetId;
/// use stellar_storage::chunk::ChunkId;
/// fn wants_asset(id: AssetId) {}
/// wants_asset(ChunkId::from_bytes([0u8; 16])); // ChunkId ≠ AssetId — 컴파일 실패
/// ```
///
/// ```compile_fail
/// use stellar_assets::id::AssetId;
/// use stellar_core_types::ids::StableId;
/// fn wants_asset(id: AssetId) {}
/// wants_asset(StableId::NIL); // StableId ≠ AssetId — 컴파일 실패(명시 변환 필요)
/// ```
///
/// 반대 방향도 마찬가지입니다.
///
/// ```compile_fail
/// use stellar_assets::id::AssetId;
/// use stellar_storage::chunk::ChunkId;
/// fn wants_chunk(id: ChunkId) {}
/// wants_chunk(AssetId::from_bytes([0u8; 16])); // AssetId ≠ ChunkId — 컴파일 실패
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetId(StableId);

impl AssetId {
    /// binary 인코딩 폭(byte). StableId와 같습니다.
    pub const ENCODED_LEN: usize = 16;

    /// nil 식별자(StableId nil).
    pub const NIL: AssetId = AssetId(StableId::NIL);

    /// StableId를 에셋 식별자로 감쌉니다.
    pub const fn from_stable_id(id: StableId) -> AssetId {
        AssetId(id)
    }

    /// 감싼 StableId.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }

    /// StableId 16바이트(big-endian) 표현으로부터 만듭니다.
    pub const fn from_bytes(bytes: [u8; 16]) -> AssetId {
        AssetId(StableId::from_bytes(bytes))
    }

    /// StableId 16바이트(big-endian) 표현을 반환합니다.
    pub const fn to_bytes(&self) -> [u8; 16] {
        self.0.to_bytes()
    }

    /// canonical 인코딩: 16바이트 raw(big-endian, 길이 프리픽스 없음).
    ///
    /// manifest·pointer의 StableId·ChunkId 필드와 같은 규칙입니다.
    pub fn encode_into(&self, enc: &mut CanonicalEncoder) {
        enc.put_fixed_raw(&self.to_bytes());
    }

    /// decoder 현재 위치에서 16바이트 raw를 읽습니다. 입력이 부족하면 오류입니다.
    pub fn decode_from(dec: &mut CanonicalDecoder<'_>) -> Result<AssetId, DecodeError> {
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(dec.take_raw(Self::ENCODED_LEN)?);
        Ok(AssetId::from_bytes(bytes))
    }
}

impl From<StableId> for AssetId {
    fn from(id: StableId) -> AssetId {
        AssetId(id)
    }
}

impl From<AssetId> for StableId {
    fn from(id: AssetId) -> StableId {
        id.0
    }
}

impl fmt::Display for AssetId {
    /// 표준 소문자 hyphen UUID(StableId 문자열 형식).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl fmt::Debug for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssetId({self})")
    }
}

impl FromStr for AssetId {
    type Err = ParseAssetIdError;

    /// 표준 hyphen UUID(대소문자 무관, StableId 규칙)를 파싱합니다. 출력은 항상 소문자입니다.
    fn from_str(s: &str) -> Result<AssetId, ParseAssetIdError> {
        s.parse::<StableId>().map(AssetId)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UUIDv7 레이아웃 고정 sample (ts=0x01923c4d5e6f, ver=7, variant=0b10).
    const SAMPLE_BYTES: [u8; 16] = [
        0x01, 0x92, 0x3c, 0x4d, 0x5e, 0x6f, 0x7a, 0xbc, 0x81, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd,
        0xef,
    ];
    const SAMPLE_TEXT: &str = "01923c4d-5e6f-7abc-8123-456789abcdef";

    fn sample() -> AssetId {
        AssetId::from_bytes(SAMPLE_BYTES)
    }

    #[test]
    fn binary_round_trip() {
        let id = sample();
        assert_eq!(id.to_bytes(), SAMPLE_BYTES);
        assert_eq!(AssetId::from_bytes(id.to_bytes()), id);
    }

    #[test]
    fn wraps_stable_id_without_changing_representation() {
        let stable = StableId::new_v7(0x0192_3c4d_5e6f, 0xabc, 0x0123_4567_89ab_cdef);
        assert_eq!(
            stable.to_bytes(),
            SAMPLE_BYTES,
            "sample은 StableId::new_v7 출력"
        );
        let id = AssetId::from_stable_id(stable);
        assert_eq!(id, sample());
        assert_eq!(id.stable_id(), stable);
        assert_eq!(AssetId::from(stable), id);
        assert_eq!(StableId::from(id), stable);
        assert_eq!(id.to_bytes(), stable.to_bytes());
        assert_eq!(id.to_string(), stable.to_string());
        assert_eq!(AssetId::NIL.stable_id(), StableId::NIL);
        let other = StableId::new_v7(1, 0, 0);
        assert_eq!(
            AssetId::from_stable_id(stable).cmp(&AssetId::from_stable_id(other)),
            stable.cmp(&other)
        );
    }

    #[test]
    fn canonical_encoding_is_16_raw_bytes_and_round_trips() {
        let id = sample();
        let mut enc = CanonicalEncoder::new();
        id.encode_into(&mut enc);
        let bytes = enc.into_bytes();
        assert_eq!(
            bytes,
            SAMPLE_BYTES.to_vec(),
            "길이 프리픽스 없는 big-endian raw"
        );

        let mut dec = CanonicalDecoder::new(&bytes);
        assert_eq!(AssetId::decode_from(&mut dec).unwrap(), id);
        dec.expect_end().unwrap();
    }

    #[test]
    fn truncated_binary_is_rejected() {
        for cut in 0..AssetId::ENCODED_LEN {
            let mut dec = CanonicalDecoder::new(&SAMPLE_BYTES[..cut]);
            assert_eq!(
                AssetId::decode_from(&mut dec).unwrap_err(),
                DecodeError::UnexpectedEof {
                    needed: 16,
                    remaining: cut
                },
                "절단 길이 {cut}"
            );
        }
    }

    #[test]
    fn string_round_trip_uses_stable_id_format() {
        let id = sample();
        let text = id.to_string();
        assert_eq!(text, SAMPLE_TEXT, "표준 소문자 hyphen UUID");
        let parsed: AssetId = text.parse().unwrap();
        assert_eq!(parsed, id);
        assert_eq!(parsed.to_string(), text);
        assert_eq!(format!("{id:?}"), format!("AssetId({SAMPLE_TEXT})"));
    }

    #[test]
    fn uppercase_input_parses_and_prints_lowercase() {
        let parsed: AssetId = SAMPLE_TEXT.to_ascii_uppercase().parse().unwrap();
        assert_eq!(parsed, sample());
        assert_eq!(parsed.to_string(), SAMPLE_TEXT);
    }

    #[test]
    fn malformed_strings_are_rejected_like_stable_id() {
        // 길이 오류와 형식 오류는 StableId 파서와 같은 오류 값을 냅니다.
        let short = "01923c4d-5e6f-7abc-8123-456789abcde";
        let long = "01923c4d-5e6f-7abc-8123-456789abcdef0";
        let bad_hyphen = "01923c4d_5e6f-7abc-8123-456789abcdef";
        let bad_hex = "01923c4d-5e6f-7abc-8123-456789abcdeg";
        for (input, expected) in [
            (short, ParseStableIdError::Length),
            (long, ParseStableIdError::Length),
            (bad_hyphen, ParseStableIdError::Format),
            (bad_hex, ParseStableIdError::Format),
        ] {
            assert_eq!(input.parse::<StableId>().unwrap_err(), expected, "{input}");
            assert_eq!(input.parse::<AssetId>().unwrap_err(), expected, "{input}");
        }
    }

    #[test]
    fn nil_round_trips() {
        assert_eq!(AssetId::NIL.to_bytes(), [0u8; 16]);
        let text = AssetId::NIL.to_string();
        assert_eq!(text, "00000000-0000-0000-0000-000000000000");
        assert_eq!(text.parse::<AssetId>().unwrap(), AssetId::NIL);
    }

    #[test]
    fn ordering_follows_stable_id_bytes() {
        let a = AssetId::from_bytes([0u8; 16]);
        let mut high = [0u8; 16];
        high[0] = 1;
        let b = AssetId::from_bytes(high);
        assert!(a < b);
        assert_eq!(a.cmp(&b), a.to_bytes().cmp(&b.to_bytes()));

        // UUIDv7 timestamp가 최상위라 시간 순 = 정렬 순.
        let mut early = SAMPLE_BYTES;
        early[5] = 0x6e;
        assert!(AssetId::from_bytes(early) < sample());
    }
}

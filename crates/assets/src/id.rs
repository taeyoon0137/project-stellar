//! `AssetId` — StableId 기반 에셋 식별자 (XIII.5.1).
//!
//! 에셋의 **논리** 식별자입니다. 같은 content hash를 여러 AssetId가 참조할 수 있으므로
//! (XIII.5.3 dedup) 내용 주소와 별개인 StableId 도메인으로 둡니다. Node·chunk 등 다른
//! StableId 도메인과 섞이지 않도록 newtype으로 감쌉니다.
//!
//! # 표현
//!
//! crate 의존 정책(II.2.8)상 `stellar-assets`는 `stellar-storage`만 직접 의존하므로
//! core-types의 `StableId` 타입을 이름으로 참조할 수 없습니다. 그래서 내부 값은
//! `StableId`와 **동일한 표현**인 16바이트(big-endian, UUIDv7 레이아웃)로 보관합니다.
//! - 정렬: `StableId`와 같은 byte 사전식(= UUIDv7 timestamp 순).
//! - 문자열: storage가 공개하는 StableId 문자열 codec([`ChunkId`]의 `Display`/`FromStr`)을
//!   변환에만 사용해 `StableId`와 글자 단위로 같은 형식(표준 소문자 hyphen UUID)을
//!   보장합니다. 변환 중간값은 밖으로 나가지 않으므로 chunk 도메인과 섞이지 않습니다.
//! - `StableId`와의 변환은 core-types를 의존하는 상위 crate가 16바이트로 수행합니다
//!   (`AssetId::from_bytes(id.to_bytes())`, `StableId::from_bytes(asset.to_bytes())`).

use std::fmt;
use std::str::FromStr;

use stellar_storage::canonical::decode::{CanonicalDecoder, DecodeError};
use stellar_storage::canonical::encode::CanonicalEncoder;
use stellar_storage::chunk::ChunkId;

/// `AssetId` 문자열 파싱 오류.
///
/// core-types의 `ParseStableIdError`와 같은 타입입니다(StableId 문자열 규칙을 그대로 사용).
pub type ParseAssetIdError = <ChunkId as FromStr>::Err;

/// StableId 기반 에셋 식별자.
///
/// # 다른 StableId 도메인과 혼용은 컴파일 실패
///
/// `AssetId`를 요구하는 자리에 다른 도메인 식별자(예: [`ChunkId`])를 넘길 수 없습니다.
///
/// ```compile_fail
/// use stellar_assets::id::AssetId;
/// use stellar_storage::chunk::ChunkId;
/// fn wants_asset(id: AssetId) {}
/// wants_asset(ChunkId::from_bytes([0u8; 16])); // ChunkId ≠ AssetId — 컴파일 실패
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
pub struct AssetId([u8; 16]);

impl AssetId {
    /// binary 인코딩 폭(byte). StableId와 같습니다.
    pub const ENCODED_LEN: usize = 16;

    /// 모든 바이트가 0인 nil 식별자(StableId nil과 같은 bytes).
    pub const NIL: AssetId = AssetId([0u8; 16]);

    /// StableId 16바이트(big-endian) 표현으로부터 만듭니다.
    pub const fn from_bytes(bytes: [u8; 16]) -> AssetId {
        AssetId(bytes)
    }

    /// StableId 16바이트(big-endian) 표현을 반환합니다.
    pub const fn to_bytes(&self) -> [u8; 16] {
        self.0
    }

    /// canonical 인코딩: 16바이트 raw(big-endian, 길이 프리픽스 없음).
    ///
    /// manifest·pointer의 StableId·ChunkId 필드와 같은 규칙입니다.
    pub fn encode_into(&self, enc: &mut CanonicalEncoder) {
        enc.put_fixed_raw(&self.0);
    }

    /// decoder 현재 위치에서 16바이트 raw를 읽습니다. 입력이 부족하면 오류입니다.
    pub fn decode_from(dec: &mut CanonicalDecoder<'_>) -> Result<AssetId, DecodeError> {
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(dec.take_raw(Self::ENCODED_LEN)?);
        Ok(AssetId(bytes))
    }

    /// 문자열 변환용 StableId codec(변환 전용, 외부로 노출하지 않음).
    fn stable_codec(&self) -> ChunkId {
        ChunkId::from_bytes(self.0)
    }
}

impl fmt::Display for AssetId {
    /// 표준 소문자 hyphen UUID(StableId 문자열 형식).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.stable_codec(), f)
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
        let codec: ChunkId = s.parse()?;
        Ok(AssetId(codec.to_bytes()))
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
            (short, short.parse::<ChunkId>().unwrap_err()),
            (long, long.parse::<ChunkId>().unwrap_err()),
            (bad_hyphen, bad_hyphen.parse::<ChunkId>().unwrap_err()),
            (bad_hex, bad_hex.parse::<ChunkId>().unwrap_err()),
        ] {
            assert_eq!(input.parse::<AssetId>().unwrap_err(), expected, "{input}");
        }
        assert_ne!(
            short.parse::<AssetId>().unwrap_err(),
            bad_hex.parse::<AssetId>().unwrap_err(),
            "길이 오류와 형식 오류는 구분됩니다"
        );
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

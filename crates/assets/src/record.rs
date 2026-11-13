//! `AssetRecord { content_hash, media_type, size, source_metadata }` (XIII.5.1).
//!
//! 한 에셋의 내용 주소(ADR-0004 content hash)와 해석 정보를 담습니다. 같은 content hash를
//! 여러 AssetId가 참조할 수 있으므로(XIII.5.3) AssetId는 record 밖의 key로 둡니다.
//!
//! # binary 형식 (canonical tagged record, R6)
//!
//! `u32` LE field 수 + tag 오름차순 field(`u32` LE tag, `u8` flags, 길이 프리픽스 payload).
//! payload는 각 값의 canonical 인코딩이며, 디코딩 시 payload 끝까지 정확히 소비해야 합니다.
//!
//! | tag | field | mandatory | payload |
//! |---|---|---|---|
//! | 1 | content hash | 예 | 길이 프리픽스 UTF-8 (`sha256:<hex>`, ADR-0004) |
//! | 2 | media type | 예 | 길이 프리픽스 UTF-8 (소문자 `type/subtype`) |
//! | 3 | size | 예 | `u64` LE (byte) |
//! | 4 | source metadata | 아니오 | 아래 nested record. `None`이면 field 생략 |
//!
//! source metadata nested record:
//!
//! | tag | field | mandatory | payload |
//! |---|---|---|---|
//! | 1 | original path | 예 | 길이 프리픽스 UTF-8 |
//!
//! # 엄격한 디코딩
//!
//! 절단·끝 잔여 bytes·tag 중복/역순·정의되지 않은 flags·unknown mandatory tag·필수 field
//! 누락·known tag의 mandatory flag 불일치·비canonical media type은 모두 거부합니다.
//! unknown **optional** tag는 R6 정책대로 무시합니다(재기록 시 결정론적으로 제거됨).

use std::fmt;

use stellar_storage::canonical::decode::{CanonicalDecoder, DecodeError};
use stellar_storage::canonical::encode::{CanonicalEncoder, EncodeError, Field};
use stellar_storage::hash::{ContentHash, ParseContentHashError};

use crate::media_type::{MediaType, MediaTypeError};

/// AssetRecord field tag.
const TAG_CONTENT_HASH: u32 = 1;
const TAG_MEDIA_TYPE: u32 = 2;
const TAG_SIZE: u32 = 3;
const TAG_SOURCE_METADATA: u32 = 4;

/// SourceMetadata field tag.
const TAG_ORIGINAL_PATH: u32 = 1;

/// known field의 (tag, mandatory flag).
const ASSET_RECORD_FIELDS: [(u32, bool); 4] = [
    (TAG_CONTENT_HASH, true),
    (TAG_MEDIA_TYPE, true),
    (TAG_SIZE, true),
    (TAG_SOURCE_METADATA, false),
];
const SOURCE_METADATA_FIELDS: [(u32, bool); 1] = [(TAG_ORIGINAL_PATH, true)];

/// 오류가 발생한 record 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    /// [`AssetRecord`] 최상위 record.
    AssetRecord,
    /// [`SourceMetadata`] nested record.
    SourceMetadata,
}

impl fmt::Display for RecordKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordKind::AssetRecord => f.write_str("AssetRecord"),
            RecordKind::SourceMetadata => f.write_str("SourceMetadata"),
        }
    }
}

/// source metadata 형식 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceMetadataError {
    /// 원본 경로가 비어 있습니다.
    EmptyPath,
    /// 원본 경로에 NUL 문자가 있습니다.
    ContainsNul,
}

impl fmt::Display for SourceMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceMetadataError::EmptyPath => f.write_str("source 원본 경로가 비어 있습니다"),
            SourceMetadataError::ContainsNul => {
                f.write_str("source 원본 경로에 NUL 문자가 있습니다")
            }
        }
    }
}

impl std::error::Error for SourceMetadataError {}

/// AssetRecord 인코딩·디코딩 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetRecordError {
    /// canonical 디코딩 실패(절단·잔여 bytes·tag 중복/역순·unknown mandatory tag 등).
    Decode(DecodeError),
    /// canonical 인코딩 실패.
    Encode(EncodeError),
    /// 필수 field가 없습니다.
    MissingField {
        /// field가 빠진 record.
        record: RecordKind,
        /// 누락된 tag.
        tag: u32,
    },
    /// known tag의 mandatory flag가 형식 정의와 다릅니다(비canonical).
    FlagMismatch {
        /// 위반이 발견된 record.
        record: RecordKind,
        /// 위반 tag.
        tag: u32,
        /// 입력에 기록된 mandatory flag.
        mandatory: bool,
    },
    /// content hash 문자열이 ADR-0004 형식이 아닙니다.
    InvalidContentHash(ParseContentHashError),
    /// media type 형식이 잘못되었습니다.
    InvalidMediaType(MediaTypeError),
    /// media type이 canonical(소문자) 형태가 아닙니다.
    NonCanonicalMediaType {
        /// 입력에 기록된 문자열.
        found: String,
    },
    /// source metadata 값이 잘못되었습니다.
    InvalidSourceMetadata(SourceMetadataError),
}

impl fmt::Display for AssetRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetRecordError::Decode(e) => write!(f, "asset record 디코딩 실패: {e}"),
            AssetRecordError::Encode(e) => write!(f, "asset record 인코딩 실패: {e}"),
            AssetRecordError::MissingField { record, tag } => {
                write!(f, "{record}의 필수 field(tag {tag})가 없습니다")
            }
            AssetRecordError::FlagMismatch {
                record,
                tag,
                mandatory,
            } => write!(
                f,
                "{record} field(tag {tag})의 mandatory flag({mandatory})가 형식 정의와 다릅니다"
            ),
            AssetRecordError::InvalidContentHash(e) => {
                write!(f, "content hash 형식이 잘못되었습니다: {e}")
            }
            AssetRecordError::InvalidMediaType(e) => {
                write!(f, "media type 형식이 잘못되었습니다: {e}")
            }
            AssetRecordError::NonCanonicalMediaType { found } => {
                write!(
                    f,
                    "media type이 canonical(소문자) 형태가 아닙니다: {found:?}"
                )
            }
            AssetRecordError::InvalidSourceMetadata(e) => {
                write!(f, "source metadata가 잘못되었습니다: {e}")
            }
        }
    }
}

impl std::error::Error for AssetRecordError {}

impl From<DecodeError> for AssetRecordError {
    fn from(e: DecodeError) -> AssetRecordError {
        AssetRecordError::Decode(e)
    }
}

impl From<EncodeError> for AssetRecordError {
    fn from(e: EncodeError) -> AssetRecordError {
        AssetRecordError::Encode(e)
    }
}

/// 외부 원본 출처 정보(optional).
///
/// import 당시 원본 경로를 기록할 뿐이며, 외부 파일 변경을 자동 반영하지 않습니다
/// (`Update from Source`는 명시 실행, XIII.5.5). canonical 문자열은 UTF-8이므로 UTF-8로
/// 표현할 수 없는 경로는 기록하지 않는 것은 호출자(import) 책임입니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceMetadata {
    original_path: String,
}

impl SourceMetadata {
    /// 원본 경로로 만듭니다. 빈 경로와 NUL 포함 경로는 거부합니다.
    pub fn new(original_path: impl Into<String>) -> Result<SourceMetadata, SourceMetadataError> {
        let original_path = original_path.into();
        if original_path.is_empty() {
            return Err(SourceMetadataError::EmptyPath);
        }
        if original_path.contains('\0') {
            return Err(SourceMetadataError::ContainsNul);
        }
        Ok(SourceMetadata { original_path })
    }

    /// import 당시 원본 경로.
    pub fn original_path(&self) -> &str {
        &self.original_path
    }

    /// nested record payload bytes.
    fn encode_payload(&self) -> Result<Vec<u8>, AssetRecordError> {
        let fields = [Field {
            tag: TAG_ORIGINAL_PATH,
            mandatory: true,
            payload: string_payload(&self.original_path)?,
        }];
        let mut enc = CanonicalEncoder::new();
        enc.put_fields(&fields)?;
        Ok(enc.into_bytes())
    }

    /// nested record payload를 끝까지 엄격히 디코딩합니다.
    fn decode_payload(payload: &[u8]) -> Result<SourceMetadata, AssetRecordError> {
        let kind = RecordKind::SourceMetadata;
        let mut dec = CanonicalDecoder::new(payload);
        let fields = take_known_fields(&mut dec, kind, &SOURCE_METADATA_FIELDS)?;
        dec.expect_end()?;
        let path = take_string_payload(required_payload(&fields, kind, TAG_ORIGINAL_PATH)?)?;
        SourceMetadata::new(path).map_err(AssetRecordError::InvalidSourceMetadata)
    }
}

/// 에셋 record.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AssetRecord {
    /// 에셋 content hash (ADR-0004). `assets/` 저장 경로의 주소입니다.
    pub content_hash: ContentHash,
    /// media type.
    pub media_type: MediaType,
    /// content 크기(byte).
    pub size: u64,
    /// 외부 원본 출처 정보(optional).
    pub source_metadata: Option<SourceMetadata>,
}

impl AssetRecord {
    /// encoder 현재 위치에 record를 canonical 인코딩합니다.
    pub fn encode_into(&self, enc: &mut CanonicalEncoder) -> Result<(), AssetRecordError> {
        let mut size = CanonicalEncoder::new();
        size.put_u64(self.size);
        let mut fields = vec![
            Field {
                tag: TAG_CONTENT_HASH,
                mandatory: true,
                payload: string_payload(&self.content_hash.to_string())?,
            },
            Field {
                tag: TAG_MEDIA_TYPE,
                mandatory: true,
                payload: string_payload(self.media_type.as_str())?,
            },
            Field {
                tag: TAG_SIZE,
                mandatory: true,
                payload: size.into_bytes(),
            },
        ];
        if let Some(source) = &self.source_metadata {
            fields.push(Field {
                tag: TAG_SOURCE_METADATA,
                mandatory: false,
                payload: source.encode_payload()?,
            });
        }
        enc.put_fields(&fields)?;
        Ok(())
    }

    /// 단독 bytes로 인코딩합니다. 같은 record는 항상 같은 bytes를 만듭니다.
    pub fn encode(&self) -> Result<Vec<u8>, AssetRecordError> {
        let mut enc = CanonicalEncoder::new();
        self.encode_into(&mut enc)?;
        Ok(enc.into_bytes())
    }

    /// decoder 현재 위치에서 record 하나를 엄격히 디코딩합니다.
    ///
    /// record 뒤에 다른 값이 이어질 수 있으므로 끝 검사는 호출자가 합니다
    /// (단독 bytes는 [`AssetRecord::decode`]).
    pub fn decode_from(dec: &mut CanonicalDecoder<'_>) -> Result<AssetRecord, AssetRecordError> {
        let kind = RecordKind::AssetRecord;
        let fields = take_known_fields(dec, kind, &ASSET_RECORD_FIELDS)?;

        let hash_text = take_string_payload(required_payload(&fields, kind, TAG_CONTENT_HASH)?)?;
        let content_hash: ContentHash = hash_text
            .parse()
            .map_err(AssetRecordError::InvalidContentHash)?;

        let media_text = take_string_payload(required_payload(&fields, kind, TAG_MEDIA_TYPE)?)?;
        let media_type = MediaType::new(media_text).map_err(AssetRecordError::InvalidMediaType)?;
        if media_type.as_str() != media_text {
            return Err(AssetRecordError::NonCanonicalMediaType {
                found: media_text.to_string(),
            });
        }

        let mut size_dec = CanonicalDecoder::new(required_payload(&fields, kind, TAG_SIZE)?);
        let size = size_dec.take_u64()?;
        size_dec.expect_end()?;

        let source_metadata = match find_payload(&fields, TAG_SOURCE_METADATA) {
            Some(payload) => Some(SourceMetadata::decode_payload(payload)?),
            None => None,
        };

        Ok(AssetRecord {
            content_hash,
            media_type,
            size,
            source_metadata,
        })
    }

    /// 단독 bytes에서 record를 디코딩합니다. 끝에 남은 bytes가 있으면 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<AssetRecord, AssetRecordError> {
        let mut dec = CanonicalDecoder::new(bytes);
        let record = AssetRecord::decode_from(&mut dec)?;
        dec.expect_end()?;
        Ok(record)
    }
}

/// 문자열 값의 canonical payload(`u32` LE 길이 프리픽스 + UTF-8).
fn string_payload(value: &str) -> Result<Vec<u8>, EncodeError> {
    let mut enc = CanonicalEncoder::new();
    enc.put_string(value)?;
    Ok(enc.into_bytes())
}

/// 문자열 payload를 끝까지 정확히 읽습니다.
fn take_string_payload(payload: &[u8]) -> Result<&str, DecodeError> {
    let mut dec = CanonicalDecoder::new(payload);
    let value = dec.take_string()?;
    dec.expect_end()?;
    Ok(value)
}

/// record를 읽고 known tag의 mandatory flag가 형식 정의와 같은지 검사합니다.
///
/// unknown mandatory tag는 storage decoder가 거부하고, unknown optional tag는 skip됩니다(R6).
fn take_known_fields(
    dec: &mut CanonicalDecoder<'_>,
    record: RecordKind,
    specs: &[(u32, bool)],
) -> Result<Vec<Field>, AssetRecordError> {
    let tags: Vec<u32> = specs.iter().map(|(tag, _)| *tag).collect();
    let fields = dec.take_fields(&tags)?;
    for field in &fields {
        let expected = specs
            .iter()
            .find(|(tag, _)| *tag == field.tag)
            .map(|(_, mandatory)| *mandatory);
        if expected != Some(field.mandatory) {
            return Err(AssetRecordError::FlagMismatch {
                record,
                tag: field.tag,
                mandatory: field.mandatory,
            });
        }
    }
    Ok(fields)
}

fn find_payload(fields: &[Field], tag: u32) -> Option<&[u8]> {
    fields
        .iter()
        .find(|f| f.tag == tag)
        .map(|f| f.payload.as_slice())
}

fn required_payload(
    fields: &[Field],
    record: RecordKind,
    tag: u32,
) -> Result<&[u8], AssetRecordError> {
    find_payload(fields, tag).ok_or(AssetRecordError::MissingField { record, tag })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(source: Option<&str>) -> AssetRecord {
        AssetRecord {
            content_hash: ContentHash::compute(b"asset payload"),
            media_type: MediaType::new("image/png").unwrap(),
            size: 1234,
            source_metadata: source.map(|p| SourceMetadata::new(p).unwrap()),
        }
    }

    fn field(tag: u32, mandatory: bool, payload: Vec<u8>) -> Field {
        Field {
            tag,
            mandatory,
            payload,
        }
    }

    fn u64_payload(value: u64) -> Vec<u8> {
        let mut enc = CanonicalEncoder::new();
        enc.put_u64(value);
        enc.into_bytes()
    }

    /// 정상 record의 field 목록(테스트에서 일부를 바꿔 손상 입력을 만듭니다).
    fn base_fields() -> Vec<Field> {
        vec![
            field(
                TAG_CONTENT_HASH,
                true,
                string_payload(&ContentHash::compute(b"asset payload").to_string()).unwrap(),
            ),
            field(TAG_MEDIA_TYPE, true, string_payload("image/png").unwrap()),
            field(TAG_SIZE, true, u64_payload(1234)),
        ]
    }

    fn encode_fields(fields: &[Field]) -> Vec<u8> {
        let mut enc = CanonicalEncoder::new();
        enc.put_fields(fields).unwrap();
        enc.into_bytes()
    }

    /// 정렬·중복 검사 없이 field를 입력 순서 그대로 기록합니다(손상 입력 생성용).
    fn raw_record(fields: &[(u32, u8, Vec<u8>)]) -> Vec<u8> {
        let mut enc = CanonicalEncoder::new();
        enc.put_u32(fields.len() as u32);
        for (tag, flags, payload) in fields {
            enc.put_u32(*tag);
            enc.put_u8(*flags);
            enc.put_bytes(payload).unwrap();
        }
        enc.into_bytes()
    }

    #[test]
    fn round_trip_without_source_metadata() {
        let original = record(None);
        let bytes = original.encode().unwrap();
        let decoded = AssetRecord::decode(&bytes).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(
            decoded.encode().unwrap(),
            bytes,
            "canonical 재기록 동일 bytes"
        );
    }

    #[test]
    fn round_trip_with_source_metadata() {
        let original = record(Some("/Users/디자이너/에셋/로고 최종.png"));
        let bytes = original.encode().unwrap();
        let decoded = AssetRecord::decode(&bytes).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(
            decoded.source_metadata.as_ref().unwrap().original_path(),
            "/Users/디자이너/에셋/로고 최종.png"
        );
        assert_eq!(decoded.encode().unwrap(), bytes);
    }

    #[test]
    fn size_boundaries_round_trip() {
        for size in [0, 1, u32::MAX as u64 + 1, u64::MAX] {
            let mut original = record(None);
            original.size = size;
            let decoded = AssetRecord::decode(&original.encode().unwrap()).unwrap();
            assert_eq!(decoded.size, size);
            assert_eq!(decoded, original);
        }
    }

    #[test]
    fn records_in_sequence_round_trip() {
        let records: Vec<AssetRecord> = (0..5u64)
            .map(|i| AssetRecord {
                content_hash: ContentHash::compute(format!("asset-{i}").as_bytes()),
                media_type: MediaType::new("font/woff2").unwrap(),
                size: i * 1000,
                source_metadata: (i % 2 == 0)
                    .then(|| SourceMetadata::new(format!("/fonts/{i}.woff2")).unwrap()),
            })
            .collect();
        let mut enc = CanonicalEncoder::new();
        for r in &records {
            r.encode_into(&mut enc).unwrap();
        }
        let bytes = enc.into_bytes();
        let mut dec = CanonicalDecoder::new(&bytes);
        let decoded: Vec<AssetRecord> = (0..5)
            .map(|_| AssetRecord::decode_from(&mut dec).unwrap())
            .collect();
        dec.expect_end().unwrap();
        assert_eq!(decoded, records);
    }

    #[test]
    fn encoding_matches_documented_layout() {
        // golden: tag 오름차순 tagged record, 정수는 LE, payload는 값의 canonical 인코딩.
        let hash = ContentHash::compute(b"asset payload").to_string();
        assert_eq!(hash.len(), 71);
        let mut expected: Vec<u8> = vec![0x03, 0x00, 0x00, 0x00]; // field 수 3
        expected.extend_from_slice(&[0x01, 0x00, 0x00, 0x00, 0x01]); // tag 1, mandatory
        expected.extend_from_slice(&[0x4b, 0x00, 0x00, 0x00]); // payload 75 bytes
        expected.extend_from_slice(&[0x47, 0x00, 0x00, 0x00]); // 문자열 71 bytes
        expected.extend_from_slice(hash.as_bytes());
        expected.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x01]); // tag 2, mandatory
        expected.extend_from_slice(&[0x0d, 0x00, 0x00, 0x00]); // payload 13 bytes
        expected.extend_from_slice(&[0x09, 0x00, 0x00, 0x00]); // 문자열 9 bytes
        expected.extend_from_slice(b"image/png");
        expected.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0x01]); // tag 3, mandatory
        expected.extend_from_slice(&[0x08, 0x00, 0x00, 0x00]); // payload 8 bytes
        expected.extend_from_slice(&[0xd2, 0x04, 0, 0, 0, 0, 0, 0]); // 1234 LE
        assert_eq!(record(None).encode().unwrap(), expected);

        // source metadata는 optional(flags 0) tag 4로 끝에 붙습니다.
        let with_source = record(Some("/a.png")).encode().unwrap();
        let mut expected_source = expected.clone();
        expected_source[0] = 0x04; // field 수 4
        expected_source.extend_from_slice(&[0x04, 0x00, 0x00, 0x00, 0x00]); // tag 4, optional
        expected_source.extend_from_slice(&[0x17, 0x00, 0x00, 0x00]); // payload 23 bytes
        expected_source.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // nested field 수 1
        expected_source.extend_from_slice(&[0x01, 0x00, 0x00, 0x00, 0x01]); // tag 1, mandatory
        expected_source.extend_from_slice(&[0x0a, 0x00, 0x00, 0x00]); // payload 10 bytes
        expected_source.extend_from_slice(&[0x06, 0x00, 0x00, 0x00]); // 문자열 6 bytes
        expected_source.extend_from_slice(b"/a.png");
        assert_eq!(with_source, expected_source);
    }

    #[test]
    fn truncated_input_is_rejected_at_every_length() {
        let bytes = record(Some("/src/logo.png")).encode().unwrap();
        for cut in 0..bytes.len() {
            let err = AssetRecord::decode(&bytes[..cut]).unwrap_err();
            assert!(
                matches!(err, AssetRecordError::Decode(_)),
                "절단 길이 {cut}: {err:?}"
            );
        }
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = record(None).encode().unwrap();
        bytes.push(0x00);
        assert_eq!(
            AssetRecord::decode(&bytes).unwrap_err(),
            AssetRecordError::Decode(DecodeError::TrailingBytes { remaining: 1 })
        );
    }

    #[test]
    fn unknown_mandatory_tag_is_rejected() {
        let mut fields = base_fields();
        fields.push(field(9, true, vec![0xaa]));
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::Decode(DecodeError::UnknownMandatoryTag { tag: 9 })
        );
    }

    #[test]
    fn unknown_optional_tag_is_skipped_per_r6() {
        let mut fields = base_fields();
        fields.push(field(9, false, vec![0xaa]));
        let bytes = encode_fields(&fields);
        let decoded = AssetRecord::decode(&bytes).unwrap();
        assert_eq!(decoded, record(None));
        // 무시한 field는 재기록에서 결정론적으로 제거됩니다.
        assert_eq!(decoded.encode().unwrap(), record(None).encode().unwrap());
    }

    #[test]
    fn nested_unknown_mandatory_tag_is_rejected() {
        let source = encode_fields(&[
            field(TAG_ORIGINAL_PATH, true, string_payload("/a.png").unwrap()),
            field(5, true, Vec::new()),
        ]);
        let mut fields = base_fields();
        fields.push(field(TAG_SOURCE_METADATA, false, source));
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::Decode(DecodeError::UnknownMandatoryTag { tag: 5 })
        );
    }

    #[test]
    fn missing_mandatory_fields_are_rejected() {
        for missing in [TAG_CONTENT_HASH, TAG_MEDIA_TYPE, TAG_SIZE] {
            let fields: Vec<Field> = base_fields()
                .into_iter()
                .filter(|f| f.tag != missing)
                .collect();
            assert_eq!(
                AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
                AssetRecordError::MissingField {
                    record: RecordKind::AssetRecord,
                    tag: missing
                }
            );
        }

        let mut fields = base_fields();
        fields.push(field(TAG_SOURCE_METADATA, false, encode_fields(&[])));
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::MissingField {
                record: RecordKind::SourceMetadata,
                tag: TAG_ORIGINAL_PATH
            }
        );
    }

    #[test]
    fn flag_mismatch_on_known_tag_is_rejected() {
        let mut fields = base_fields();
        fields[2].mandatory = false;
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::FlagMismatch {
                record: RecordKind::AssetRecord,
                tag: TAG_SIZE,
                mandatory: false
            }
        );

        let mut fields = base_fields();
        let source = SourceMetadata::new("/a.png").unwrap();
        fields.push(field(
            TAG_SOURCE_METADATA,
            true,
            source.encode_payload().unwrap(),
        ));
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::FlagMismatch {
                record: RecordKind::AssetRecord,
                tag: TAG_SOURCE_METADATA,
                mandatory: true
            }
        );
    }

    #[test]
    fn duplicate_misordered_and_bad_flag_tags_are_rejected() {
        let fields = base_fields();
        let entry = |i: usize, flags: u8| (fields[i].tag, flags, fields[i].payload.clone());

        let duplicate = raw_record(&[entry(0, 1), entry(0, 1), entry(1, 1), entry(2, 1)]);
        assert_eq!(
            AssetRecord::decode(&duplicate).unwrap_err(),
            AssetRecordError::Decode(DecodeError::DuplicateTag { tag: 1 })
        );

        let misordered = raw_record(&[entry(1, 1), entry(0, 1), entry(2, 1)]);
        assert_eq!(
            AssetRecord::decode(&misordered).unwrap_err(),
            AssetRecordError::Decode(DecodeError::MisorderedTag { tag: 1 })
        );

        let bad_flags = raw_record(&[entry(0, 0x03), entry(1, 1), entry(2, 1)]);
        assert_eq!(
            AssetRecord::decode(&bad_flags).unwrap_err(),
            AssetRecordError::Decode(DecodeError::InvalidFlags { value: 0x03 })
        );
    }

    #[test]
    fn invalid_content_hash_is_rejected() {
        let mut fields = base_fields();
        fields[0].payload = string_payload("sha256:abcd").unwrap();
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::InvalidContentHash(ParseContentHashError::InvalidLength {
                expected: 64,
                actual: 4
            })
        );
    }

    #[test]
    fn invalid_and_non_canonical_media_type_are_rejected() {
        let mut fields = base_fields();
        fields[1].payload = string_payload("imagepng").unwrap();
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::InvalidMediaType(MediaTypeError::MissingSeparator)
        );

        fields[1].payload = string_payload("Image/PNG").unwrap();
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::NonCanonicalMediaType {
                found: "Image/PNG".to_string()
            }
        );
    }

    #[test]
    fn field_payload_must_be_consumed_exactly() {
        // size payload 9 bytes(잔여 1) / 7 bytes(부족).
        let mut fields = base_fields();
        let mut long = u64_payload(1234);
        long.push(0);
        fields[2].payload = long;
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::Decode(DecodeError::TrailingBytes { remaining: 1 })
        );
        fields[2].payload = u64_payload(1234)[..7].to_vec();
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::Decode(DecodeError::UnexpectedEof {
                needed: 8,
                remaining: 7
            })
        );

        // 문자열 payload 뒤 잔여 bytes.
        let mut fields = base_fields();
        fields[1].payload.push(b'x');
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::Decode(DecodeError::TrailingBytes { remaining: 1 })
        );

        // nested source record 뒤 잔여 bytes.
        let mut source = SourceMetadata::new("/a.png")
            .unwrap()
            .encode_payload()
            .unwrap();
        source.push(0);
        let mut fields = base_fields();
        fields.push(field(TAG_SOURCE_METADATA, false, source));
        assert_eq!(
            AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
            AssetRecordError::Decode(DecodeError::TrailingBytes { remaining: 1 })
        );
    }

    #[test]
    fn invalid_source_metadata_is_rejected() {
        assert_eq!(
            SourceMetadata::new("").unwrap_err(),
            SourceMetadataError::EmptyPath
        );
        assert_eq!(
            SourceMetadata::new("/a\0b").unwrap_err(),
            SourceMetadataError::ContainsNul
        );

        for (path, expected) in [
            ("", SourceMetadataError::EmptyPath),
            ("/a\0b", SourceMetadataError::ContainsNul),
        ] {
            let source = encode_fields(&[field(
                TAG_ORIGINAL_PATH,
                true,
                string_payload(path).unwrap(),
            )]);
            let mut fields = base_fields();
            fields.push(field(TAG_SOURCE_METADATA, false, source));
            assert_eq!(
                AssetRecord::decode(&encode_fields(&fields)).unwrap_err(),
                AssetRecordError::InvalidSourceMetadata(expected)
            );
        }
    }

    #[test]
    fn same_content_hash_can_back_distinct_records() {
        // dedup(XIII.5.3) 전제: record는 AssetId를 포함하지 않고 content hash만 공유합니다.
        let a = record(Some("/a.png"));
        let b = record(Some("/copy/a.png"));
        assert_eq!(a.content_hash, b.content_hash);
        assert_ne!(a.encode().unwrap(), b.encode().unwrap());
    }

    #[test]
    fn error_messages_are_descriptive() {
        let err = AssetRecordError::MissingField {
            record: RecordKind::SourceMetadata,
            tag: 1,
        };
        assert_eq!(
            err.to_string(),
            "SourceMetadata의 필수 field(tag 1)가 없습니다"
        );
        let err = AssetRecordError::Decode(DecodeError::TrailingBytes { remaining: 2 });
        assert!(err.to_string().contains("2 bytes"));
    }
}

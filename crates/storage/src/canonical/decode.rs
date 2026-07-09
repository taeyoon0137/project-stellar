//! canonical decoder (V.1.3).
//!
//! [`CanonicalEncoder`](super::encode::CanonicalEncoder)가 기록한 bytes를 읽습니다.
//! malformed 입력은 panic 없이 [`DecodeError`]로 거부합니다:
//! - 길이 프리픽스가 남은 입력보다 큰 length overflow.
//! - map 중복 key와 canonical 정렬 위반(R4).
//! - 레코드의 unknown mandatory tag(R6). unknown optional tag는 skip합니다.
//!
//! float 유한성 정책(R5)은 V.1.4에서 encoder와 함께 확정합니다.

use std::fmt;

use super::encode::{Field, FIELD_FLAG_MANDATORY};
use super::MapEntry;

/// canonical 디코딩 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// 남은 입력이 필요한 길이보다 짧습니다.
    UnexpectedEof {
        /// 필요한 byte 수.
        needed: usize,
        /// 남은 byte 수.
        remaining: usize,
    },
    /// 길이 프리픽스가 남은 입력을 초과합니다(length overflow).
    LengthOverflow {
        /// 길이 프리픽스 값.
        len: u32,
        /// 남은 byte 수.
        remaining: usize,
    },
    /// `bool` 위치에 0x00/0x01이 아닌 byte가 있습니다.
    InvalidBool {
        /// 실제 byte 값.
        value: u8,
    },
    /// 문자열 payload가 UTF-8이 아닙니다(R3).
    InvalidUtf8,
    /// map key가 중복됩니다(R4).
    DuplicateKey {
        /// 중복된 key bytes.
        key: Vec<u8>,
    },
    /// map key가 canonical byte 순서로 정렬되어 있지 않습니다(R4).
    MisorderedKey {
        /// 정렬 위반이 발견된 key bytes.
        key: Vec<u8>,
    },
    /// 레코드 field tag가 중복됩니다(R6).
    DuplicateTag {
        /// 중복된 tag.
        tag: u32,
    },
    /// 레코드 field tag가 오름차순으로 정렬되어 있지 않습니다(R6).
    MisorderedTag {
        /// 정렬 위반이 발견된 tag.
        tag: u32,
    },
    /// 디코더가 모르는 mandatory tag입니다(R6).
    UnknownMandatoryTag {
        /// 알 수 없는 tag.
        tag: u32,
    },
    /// field flags에 정의되지 않은 bit가 설정되어 있습니다.
    InvalidFlags {
        /// 실제 flags byte.
        value: u8,
    },
    /// 값을 모두 읽은 뒤 입력이 남아 있습니다.
    TrailingBytes {
        /// 남은 byte 수.
        remaining: usize,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::UnexpectedEof { needed, remaining } => {
                write!(
                    f,
                    "입력이 부족합니다: {needed} bytes 필요, {remaining} bytes 남음"
                )
            }
            DecodeError::LengthOverflow { len, remaining } => {
                write!(
                    f,
                    "길이 프리픽스가 남은 입력을 초과합니다: {len} > {remaining}"
                )
            }
            DecodeError::InvalidBool { value } => {
                write!(f, "bool 값이 canonical하지 않습니다: 0x{value:02x}")
            }
            DecodeError::InvalidUtf8 => f.write_str("문자열 payload가 UTF-8이 아닙니다"),
            DecodeError::DuplicateKey { key } => {
                write!(f, "map key가 중복됩니다: {}", hex(key))
            }
            DecodeError::MisorderedKey { key } => {
                write!(f, "map key 정렬이 canonical하지 않습니다: {}", hex(key))
            }
            DecodeError::DuplicateTag { tag } => write!(f, "field tag가 중복됩니다: {tag}"),
            DecodeError::MisorderedTag { tag } => {
                write!(f, "field tag 정렬이 canonical하지 않습니다: {tag}")
            }
            DecodeError::UnknownMandatoryTag { tag } => {
                write!(f, "알 수 없는 mandatory tag입니다: {tag}")
            }
            DecodeError::InvalidFlags { value } => {
                write!(f, "정의되지 않은 field flags입니다: 0x{value:02x}")
            }
            DecodeError::TrailingBytes { remaining } => {
                write!(f, "입력 끝에 {remaining} bytes가 남았습니다")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// canonical binary decoder. 입력 slice 위 cursor로 동작합니다.
#[derive(Debug)]
pub struct CanonicalDecoder<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> CanonicalDecoder<'a> {
    /// 입력 bytes 위에 decoder를 만듭니다.
    pub fn new(input: &'a [u8]) -> CanonicalDecoder<'a> {
        CanonicalDecoder { input, pos: 0 }
    }

    /// 아직 읽지 않은 byte 수.
    pub fn remaining(&self) -> usize {
        self.input.len() - self.pos
    }

    /// 입력이 전부 소비되었는지 검사합니다. 남은 bytes가 있으면 오류입니다.
    pub fn expect_end(&self) -> Result<(), DecodeError> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes {
                remaining: self.remaining(),
            })
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < n {
            return Err(DecodeError::UnexpectedEof {
                needed: n,
                remaining: self.remaining(),
            });
        }
        let slice = &self.input[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn take_array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let slice = self.take(N)?;
        Ok(slice.try_into().expect("길이 N 보장"))
    }

    /// `u8`을 읽습니다(R2).
    pub fn take_u8(&mut self) -> Result<u8, DecodeError> {
        Ok(u8::from_le_bytes(self.take_array()?))
    }

    /// `u16` LE를 읽습니다(R1, R2).
    pub fn take_u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take_array()?))
    }

    /// `u32` LE를 읽습니다(R1, R2).
    pub fn take_u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take_array()?))
    }

    /// `u64` LE를 읽습니다(R1, R2).
    pub fn take_u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take_array()?))
    }

    /// `i8`(2의 보수)을 읽습니다(R2).
    pub fn take_i8(&mut self) -> Result<i8, DecodeError> {
        Ok(i8::from_le_bytes(self.take_array()?))
    }

    /// `i16` LE(2의 보수)를 읽습니다(R1, R2).
    pub fn take_i16(&mut self) -> Result<i16, DecodeError> {
        Ok(i16::from_le_bytes(self.take_array()?))
    }

    /// `i32` LE(2의 보수)를 읽습니다(R1, R2).
    pub fn take_i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.take_array()?))
    }

    /// `i64` LE(2의 보수)를 읽습니다(R1, R2).
    pub fn take_i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.take_array()?))
    }

    /// `bool`을 읽습니다. 0x00/0x01 외 값은 canonical 위반으로 거부합니다(R2).
    pub fn take_bool(&mut self) -> Result<bool, DecodeError> {
        match self.take_u8()? {
            0x00 => Ok(false),
            0x01 => Ok(true),
            value => Err(DecodeError::InvalidBool { value }),
        }
    }

    /// `f32` IEEE-754 비트 패턴을 LE로 읽습니다(R1). 유한성 정책은 V.1.4에서 적용합니다.
    pub fn take_f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_le_bytes(self.take_array()?))
    }

    /// `f64` IEEE-754 비트 패턴을 LE로 읽습니다(R1). 유한성 정책은 V.1.4에서 적용합니다.
    pub fn take_f64(&mut self) -> Result<f64, DecodeError> {
        Ok(f64::from_le_bytes(self.take_array()?))
    }

    /// 길이 프리픽스 바이트열을 읽습니다(R3). 길이가 남은 입력을 초과하면 거부합니다.
    pub fn take_bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let len = self.take_u32()?;
        let needed = len as usize;
        if self.remaining() < needed {
            return Err(DecodeError::LengthOverflow {
                len,
                remaining: self.remaining(),
            });
        }
        self.take(needed)
    }

    /// 길이 프리픽스 UTF-8 문자열을 읽습니다(R3).
    pub fn take_string(&mut self) -> Result<&'a str, DecodeError> {
        let bytes = self.take_bytes()?;
        std::str::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8)
    }

    /// map을 읽습니다(R4). key는 canonical byte 순서로 엄격히 증가해야 하며
    /// 중복·정렬 위반은 거부합니다.
    pub fn take_map(&mut self) -> Result<Vec<MapEntry>, DecodeError> {
        let count = self.take_u32()? as usize;
        let mut entries: Vec<MapEntry> = Vec::new();
        for _ in 0..count {
            let key = self.take_bytes()?.to_vec();
            let value = self.take_bytes()?.to_vec();
            if let Some((last_key, _)) = entries.last() {
                match key.cmp(last_key) {
                    std::cmp::Ordering::Equal => {
                        return Err(DecodeError::DuplicateKey { key });
                    }
                    std::cmp::Ordering::Less => {
                        return Err(DecodeError::MisorderedKey { key });
                    }
                    std::cmp::Ordering::Greater => {}
                }
            }
            entries.push((key, value));
        }
        Ok(entries)
    }

    /// 레코드를 읽습니다(R6). tag는 오름차순으로 엄격히 증가해야 합니다.
    ///
    /// `known_tags`에 없는 tag는 mandatory면 [`DecodeError::UnknownMandatoryTag`]로
    /// 거부하고, optional이면 결과에서 제외(skip)합니다.
    pub fn take_fields(&mut self, known_tags: &[u32]) -> Result<Vec<Field>, DecodeError> {
        let count = self.take_u32()? as usize;
        let mut fields: Vec<Field> = Vec::new();
        let mut last_tag: Option<u32> = None;
        for _ in 0..count {
            let tag = self.take_u32()?;
            let flags = self.take_u8()?;
            if flags & !FIELD_FLAG_MANDATORY != 0 {
                return Err(DecodeError::InvalidFlags { value: flags });
            }
            let mandatory = flags & FIELD_FLAG_MANDATORY != 0;
            let payload = self.take_bytes()?.to_vec();
            if let Some(last) = last_tag {
                match tag.cmp(&last) {
                    std::cmp::Ordering::Equal => {
                        return Err(DecodeError::DuplicateTag { tag });
                    }
                    std::cmp::Ordering::Less => {
                        return Err(DecodeError::MisorderedTag { tag });
                    }
                    std::cmp::Ordering::Greater => {}
                }
            }
            last_tag = Some(tag);
            if known_tags.contains(&tag) {
                fields.push(Field {
                    tag,
                    mandatory,
                    payload,
                });
            } else if mandatory {
                return Err(DecodeError::UnknownMandatoryTag { tag });
            }
            // unknown optional tag는 skip합니다(R6).
        }
        Ok(fields)
    }
}

#[cfg(test)]
mod tests {
    use super::super::encode::CanonicalEncoder;
    use super::*;

    fn from_hex(s: &str) -> Vec<u8> {
        let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        (0..clean.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn primitives_round_trip() {
        let mut enc = CanonicalEncoder::new();
        enc.put_u8(0xab);
        enc.put_u16(258);
        enc.put_u32(1);
        enc.put_u64(u64::MAX);
        enc.put_i8(-1);
        enc.put_i16(-2);
        enc.put_i32(-256);
        enc.put_i64(i64::MIN);
        enc.put_bool(true);
        enc.put_f32(1.0);
        enc.put_f64(-2.5);
        enc.put_string("héllo").unwrap();
        let bytes = enc.into_bytes();

        let mut dec = CanonicalDecoder::new(&bytes);
        assert_eq!(dec.take_u8().unwrap(), 0xab);
        assert_eq!(dec.take_u16().unwrap(), 258);
        assert_eq!(dec.take_u32().unwrap(), 1);
        assert_eq!(dec.take_u64().unwrap(), u64::MAX);
        assert_eq!(dec.take_i8().unwrap(), -1);
        assert_eq!(dec.take_i16().unwrap(), -2);
        assert_eq!(dec.take_i32().unwrap(), -256);
        assert_eq!(dec.take_i64().unwrap(), i64::MIN);
        assert!(dec.take_bool().unwrap());
        assert_eq!(dec.take_f32().unwrap(), 1.0);
        assert_eq!(dec.take_f64().unwrap(), -2.5);
        assert_eq!(dec.take_string().unwrap(), "héllo");
        dec.expect_end().unwrap();
    }

    #[test]
    fn map_round_trip_returns_sorted_entries() {
        let entries = vec![
            (b"zeta".to_vec(), vec![0x02]),
            (b"alpha".to_vec(), vec![0x01]),
        ];
        let mut enc = CanonicalEncoder::new();
        enc.put_map(&entries).unwrap();
        let bytes = enc.into_bytes();

        let mut dec = CanonicalDecoder::new(&bytes);
        let decoded = dec.take_map().unwrap();
        dec.expect_end().unwrap();
        assert_eq!(
            decoded,
            vec![
                (b"alpha".to_vec(), vec![0x01]),
                (b"zeta".to_vec(), vec![0x02]),
            ]
        );
    }

    #[test]
    fn fields_round_trip_and_unknown_optional_is_skipped() {
        let fields = vec![
            Field {
                tag: 1,
                mandatory: true,
                payload: vec![0x01],
            },
            Field {
                tag: 9,
                mandatory: false,
                payload: vec![0x09],
            },
        ];
        let mut enc = CanonicalEncoder::new();
        enc.put_fields(&fields).unwrap();
        let bytes = enc.into_bytes();

        // tag 9를 모르는 디코더: optional이므로 skip.
        let mut dec = CanonicalDecoder::new(&bytes);
        let decoded = dec.take_fields(&[1]).unwrap();
        dec.expect_end().unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].tag, 1);
        assert!(decoded[0].mandatory);
        assert_eq!(decoded[0].payload, vec![0x01]);
    }

    #[test]
    fn unknown_mandatory_tag_is_rejected() {
        let fields = vec![Field {
            tag: 42,
            mandatory: true,
            payload: vec![],
        }];
        let mut enc = CanonicalEncoder::new();
        enc.put_fields(&fields).unwrap();
        let bytes = enc.into_bytes();

        let mut dec = CanonicalDecoder::new(&bytes);
        assert_eq!(
            dec.take_fields(&[1, 2, 3]).unwrap_err(),
            DecodeError::UnknownMandatoryTag { tag: 42 }
        );
    }

    /// malformed corpus 항목이 사용할 디코드 경로.
    enum Path {
        U32,
        Bool,
        String,
        Map,
        Fields,
    }

    #[test]
    fn malformed_corpus_returns_errors_without_panic() {
        // (설명, 디코드 경로, hex) 형태의 malformed corpus.
        // 각 입력은 panic 없이 Err를 반환해야 합니다.
        let corpus: Vec<(&str, Path, Vec<u8>)> = vec![
            ("빈 입력에서 u32", Path::U32, vec![]),
            ("u32 절단", Path::U32, from_hex("0102")),
            (
                "길이 프리픽스가 입력 초과",
                Path::String,
                from_hex("ffffffff"),
            ),
            (
                "문자열 길이 4, payload 2",
                Path::String,
                from_hex("040000006869"),
            ),
            ("비UTF-8 문자열", Path::String, from_hex("02000000ffff")),
            ("bool 0x02", Path::Bool, from_hex("02")),
            ("map count 1, entry 없음", Path::Map, from_hex("01000000")),
            (
                "map key 길이가 남은 입력 초과",
                Path::Map,
                from_hex("01000000 ffffff7f 61"),
            ),
            (
                "map 중복 key",
                Path::Map,
                from_hex("02000000 01000000 61 0100000001 01000000 61 0100000002"),
            ),
            (
                "map 정렬 위반",
                Path::Map,
                from_hex("02000000 01000000 62 0100000001 01000000 61 0100000002"),
            ),
            (
                "field 중복 tag",
                Path::Fields,
                from_hex("02000000 01000000 00 00000000 01000000 00 00000000"),
            ),
            (
                "field 정렬 위반",
                Path::Fields,
                from_hex("02000000 02000000 00 00000000 01000000 00 00000000"),
            ),
            (
                "field 알 수 없는 flags",
                Path::Fields,
                from_hex("01000000 01000000 80 00000000"),
            ),
            (
                "field payload 절단",
                Path::Fields,
                from_hex("01000000 01000000 01 0400000000"),
            ),
            (
                "unknown mandatory tag",
                Path::Fields,
                from_hex("01000000 63000000 01 00000000"),
            ),
            ("map 뒤 trailing bytes", Path::Map, from_hex("00000000 ff")),
        ];

        for (name, path, bytes) in corpus {
            let known = &[1u32, 2];
            let mut dec = CanonicalDecoder::new(&bytes);
            let result: Result<(), DecodeError> = match path {
                Path::U32 => dec.take_u32().map(|_| ()),
                Path::Bool => dec.take_bool().map(|_| ()),
                Path::String => dec.take_string().map(|_| ()),
                Path::Map => dec.take_map().and_then(|_| dec.expect_end()),
                Path::Fields => dec.take_fields(known).and_then(|_| dec.expect_end()),
            };
            assert!(
                result.is_err(),
                "malformed corpus 항목이 오류가 아닙니다: {name}"
            );
        }
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut enc = CanonicalEncoder::new();
        enc.put_u8(1);
        enc.put_u8(2);
        let bytes = enc.into_bytes();
        let mut dec = CanonicalDecoder::new(&bytes);
        dec.take_u8().unwrap();
        assert_eq!(
            dec.expect_end().unwrap_err(),
            DecodeError::TrailingBytes { remaining: 1 }
        );
    }

    #[test]
    fn error_display_is_stable() {
        assert!(DecodeError::UnknownMandatoryTag { tag: 42 }
            .to_string()
            .contains("42"));
        assert!(DecodeError::LengthOverflow {
            len: 100,
            remaining: 2
        }
        .to_string()
        .contains("100"));
    }
}

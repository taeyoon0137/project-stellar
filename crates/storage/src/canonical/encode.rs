//! canonical encoder (V.1.2).
//!
//! 규칙(R1~R4):
//! - R1: 모든 다중 바이트 값은 little-endian.
//! - R2: 정수는 타입별 고정 폭(varint 아님). `bool`은 1바이트(0x00/0x01).
//! - R3: 문자열·바이트열은 `u32` LE 길이 프리픽스 + payload(UTF-8).
//! - R4: map은 key의 canonical byte 순서로 정렬해 기록합니다. 입력 삽입 순서와
//!   무관하게 동일 bytes를 만들고, 중복 key는 거부합니다.
//!
//! float 정책(R5: NaN·±Infinity 거부, `-0.0` 정규화)은 V.1.4에서 적용합니다.
//! 현재 [`CanonicalEncoder::put_f32`]/[`CanonicalEncoder::put_f64`]는 IEEE-754
//! 비트 패턴을 그대로 기록합니다.

use std::fmt;

/// canonical 인코딩 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// 길이 프리픽스가 `u32` 범위를 초과합니다(R3).
    LengthOverflow {
        /// 초과한 payload 길이(byte).
        len: usize,
    },
    /// map에 동일한 key가 두 번 이상 나타납니다(R4).
    DuplicateKey {
        /// 중복된 key bytes.
        key: Vec<u8>,
    },
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncodeError::LengthOverflow { len } => {
                write!(f, "길이 프리픽스가 u32 범위를 초과합니다: {len} bytes")
            }
            EncodeError::DuplicateKey { key } => {
                write!(f, "map key가 중복됩니다: {}", hex(key))
            }
        }
    }
}

impl std::error::Error for EncodeError {}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 길이 프리픽스로 기록 가능한 길이인지 검사합니다(R3).
fn check_len(len: usize) -> Result<u32, EncodeError> {
    u32::try_from(len).map_err(|_| EncodeError::LengthOverflow { len })
}

/// canonical binary encoder.
///
/// 같은 논리 값 입력은 항상 같은 bytes를 만듭니다. map은 입력 순서와 무관하게
/// key byte 순서로 기록합니다(R4).
#[derive(Debug, Default)]
pub struct CanonicalEncoder {
    buf: Vec<u8>,
}

impl CanonicalEncoder {
    /// 빈 encoder를 만듭니다.
    pub fn new() -> CanonicalEncoder {
        CanonicalEncoder { buf: Vec::new() }
    }

    /// 지금까지 기록된 bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }

    /// encoder를 소비하고 기록된 bytes를 반환합니다.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// `u8` 1바이트를 기록합니다(R2).
    pub fn put_u8(&mut self, value: u8) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// `u16` LE 2바이트를 기록합니다(R1, R2).
    pub fn put_u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// `u32` LE 4바이트를 기록합니다(R1, R2).
    pub fn put_u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// `u64` LE 8바이트를 기록합니다(R1, R2).
    pub fn put_u64(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// `i8` 1바이트(2의 보수)를 기록합니다(R2).
    pub fn put_i8(&mut self, value: i8) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// `i16` LE 2바이트(2의 보수)를 기록합니다(R1, R2).
    pub fn put_i16(&mut self, value: i16) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// `i32` LE 4바이트(2의 보수)를 기록합니다(R1, R2).
    pub fn put_i32(&mut self, value: i32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// `i64` LE 8바이트(2의 보수)를 기록합니다(R1, R2).
    pub fn put_i64(&mut self, value: i64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// `bool` 1바이트를 기록합니다: `0x00`=false, `0x01`=true(R2).
    pub fn put_bool(&mut self, value: bool) {
        self.buf.push(u8::from(value));
    }

    /// `f32` IEEE-754 비트 패턴을 LE 4바이트로 기록합니다(R1).
    ///
    /// 유한성 검사·`-0.0` 정규화(R5)는 V.1.4에서 적용합니다.
    pub fn put_f32(&mut self, value: f32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// `f64` IEEE-754 비트 패턴을 LE 8바이트로 기록합니다(R1).
    ///
    /// 유한성 검사·`-0.0` 정규화(R5)는 V.1.4에서 적용합니다.
    pub fn put_f64(&mut self, value: f64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// 바이트열을 `u32` LE 길이 프리픽스 + payload로 기록합니다(R3).
    pub fn put_bytes(&mut self, payload: &[u8]) -> Result<(), EncodeError> {
        let len = check_len(payload.len())?;
        self.put_u32(len);
        self.buf.extend_from_slice(payload);
        Ok(())
    }

    /// UTF-8 문자열을 `u32` LE 길이 프리픽스 + payload로 기록합니다(R3).
    ///
    /// 유니코드 정규화는 상위 계층 책임입니다(III.3.4).
    pub fn put_string(&mut self, value: &str) -> Result<(), EncodeError> {
        self.put_bytes(value.as_bytes())
    }

    /// map을 key byte 순서로 정렬해 기록합니다(R4).
    ///
    /// 형식: `u32` LE entry 수, 이어서 정렬된 각 entry의
    /// 길이 프리픽스 key bytes + 길이 프리픽스 value bytes.
    /// value bytes는 호출자가 미리 canonical 인코딩한 결과입니다.
    /// 입력 삽입 순서는 결과에 영향을 주지 않으며 중복 key는 거부합니다.
    pub fn put_map(&mut self, entries: &[(Vec<u8>, Vec<u8>)]) -> Result<(), EncodeError> {
        let count = check_len(entries.len())?;
        let mut sorted: Vec<&(Vec<u8>, Vec<u8>)> = entries.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        for pair in sorted.windows(2) {
            if pair[0].0 == pair[1].0 {
                return Err(EncodeError::DuplicateKey {
                    key: pair[0].0.clone(),
                });
            }
        }
        self.put_u32(count);
        for (key, value) in sorted {
            self.put_bytes(key)?;
            self.put_bytes(value)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// golden fixture 한 항목.
    struct Fixture {
        name: String,
        kind: String,
        value: String,
        bytes: String,
    }

    /// `docs/fixtures/storage/canonical-primitives.yaml`을 줄 단위로 파싱합니다.
    /// fixture 스키마는 `cargo xtask validate-storage-fixtures`가 별도 검증하므로
    /// 여기서는 name/type/value/bytes 4필드만 추출합니다.
    fn load_golden_fixtures() -> Vec<Fixture> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/fixtures/storage/canonical-primitives.yaml"
        );
        let text = std::fs::read_to_string(path).expect("golden fixture 파일");
        let mut fixtures: Vec<Fixture> = Vec::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("- name:") {
                fixtures.push(Fixture {
                    name: rest.trim().to_string(),
                    kind: String::new(),
                    value: String::new(),
                    bytes: String::new(),
                });
            } else if let Some(rest) = line.strip_prefix("type:") {
                fixtures.last_mut().expect("name 선행").kind = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("value:") {
                fixtures.last_mut().expect("name 선행").value = unquote(rest.trim());
            } else if let Some(rest) = line.strip_prefix("bytes:") {
                fixtures.last_mut().expect("name 선행").bytes = unquote(rest.trim());
            }
        }
        assert!(!fixtures.is_empty(), "fixture가 비어 있지 않아야 합니다");
        fixtures
    }

    fn unquote(s: &str) -> String {
        s.strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(s)
            .to_string()
    }

    /// fixture 항목 하나를 인코딩합니다.
    fn encode_fixture(fixture: &Fixture) -> Vec<u8> {
        let mut enc = CanonicalEncoder::new();
        let v = fixture.value.as_str();
        match fixture.kind.as_str() {
            "u8" => enc.put_u8(v.parse().unwrap()),
            "u16" => enc.put_u16(v.parse().unwrap()),
            "u32" => enc.put_u32(v.parse().unwrap()),
            "u64" => enc.put_u64(v.parse().unwrap()),
            "i8" => enc.put_i8(v.parse().unwrap()),
            "i16" => enc.put_i16(v.parse().unwrap()),
            "i32" => enc.put_i32(v.parse().unwrap()),
            "i64" => enc.put_i64(v.parse().unwrap()),
            "bool" => enc.put_bool(v.parse().unwrap()),
            "f32" => enc.put_f32(v.parse().unwrap()),
            "f64" => enc.put_f64(v.parse().unwrap()),
            "string" => enc.put_string(v).unwrap(),
            other => panic!("알 수 없는 fixture type: {other}"),
        }
        enc.into_bytes()
    }

    #[test]
    fn golden_primitive_fixtures_match() {
        for fixture in load_golden_fixtures() {
            let encoded = hex(&encode_fixture(&fixture));
            assert_eq!(
                encoded, fixture.bytes,
                "fixture {} 불일치(기대 {}, 실제 {})",
                fixture.name, fixture.bytes, encoded
            );
        }
    }

    #[test]
    fn integer_widths_are_fixed() {
        let mut enc = CanonicalEncoder::new();
        enc.put_u8(1);
        enc.put_u16(1);
        enc.put_u32(1);
        enc.put_u64(1);
        enc.put_i8(-1);
        enc.put_i16(-1);
        enc.put_i32(-1);
        enc.put_i64(-1);
        enc.put_bool(true);
        assert_eq!(enc.bytes().len(), 1 + 2 + 4 + 8 + 1 + 2 + 4 + 8 + 1);
    }

    #[test]
    fn strings_are_length_prefixed_utf8() {
        let mut enc = CanonicalEncoder::new();
        enc.put_string("é").unwrap();
        assert_eq!(hex(enc.bytes()), "02000000c3a9");
    }

    #[test]
    fn empty_map_is_count_zero() {
        let mut enc = CanonicalEncoder::new();
        enc.put_map(&[]).unwrap();
        assert_eq!(hex(enc.bytes()), "00000000");
    }

    #[test]
    fn map_is_sorted_by_key_bytes() {
        let entries = vec![
            (b"b".to_vec(), vec![0x02]),
            (b"a".to_vec(), vec![0x01]),
            (b"ab".to_vec(), vec![0x03]),
        ];
        let mut enc = CanonicalEncoder::new();
        enc.put_map(&entries).unwrap();
        // count=3, 그리고 "a" < "ab" < "b" (byte 순서).
        let expected = "03000000\
             0100000061 0100000001\
             02000000 6162 0100000003\
             0100000062 0100000002"
            .replace(' ', "");
        assert_eq!(hex(enc.bytes()), expected);
    }

    /// 결정적 pseudo-random 순열 생성(LCG 기반 Fisher-Yates).
    fn shuffle<T>(items: &mut [T], seed: u64) {
        let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        for i in (1..items.len()).rev() {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let j = (state >> 33) as usize % (i + 1);
            items.swap(i, j);
        }
    }

    #[test]
    fn thousand_shuffled_insert_orders_encode_identical_bytes() {
        // 검증 기준(V.1.2): 동일 값의 삽입 순서를 바꾼 1,000 fixture가 동일 bytes.
        let base: Vec<(Vec<u8>, Vec<u8>)> = (0u32..12)
            .map(|i| {
                let key = format!("key-{i:02}").into_bytes();
                let mut value = CanonicalEncoder::new();
                value.put_u32(i * 7 + 1);
                (key, value.into_bytes())
            })
            .collect();
        let mut reference = CanonicalEncoder::new();
        reference.put_map(&base).unwrap();
        let reference = reference.into_bytes();

        for seed in 0..1_000u64 {
            let mut permuted = base.clone();
            shuffle(&mut permuted, seed);
            let mut enc = CanonicalEncoder::new();
            enc.put_map(&permuted).unwrap();
            assert_eq!(
                enc.into_bytes(),
                reference,
                "seed {seed} 순열에서 bytes가 달라졌습니다"
            );
        }
    }

    #[test]
    fn duplicate_map_key_is_rejected() {
        let entries = vec![
            (b"same".to_vec(), vec![0x01]),
            (b"other".to_vec(), vec![0x02]),
            (b"same".to_vec(), vec![0x03]),
        ];
        let mut enc = CanonicalEncoder::new();
        let err = enc.put_map(&entries).unwrap_err();
        assert_eq!(
            err,
            EncodeError::DuplicateKey {
                key: b"same".to_vec()
            }
        );
    }

    #[test]
    fn length_overflow_is_rejected() {
        assert_eq!(check_len(0).unwrap(), 0);
        assert_eq!(check_len(u32::MAX as usize).unwrap(), u32::MAX);
        let too_long = u32::MAX as usize + 1;
        assert_eq!(
            check_len(too_long).unwrap_err(),
            EncodeError::LengthOverflow { len: too_long }
        );
    }

    #[test]
    fn error_display_is_stable() {
        let overflow = EncodeError::LengthOverflow { len: 5_000_000_000 };
        assert!(overflow.to_string().contains("5000000000"));
        let duplicate = EncodeError::DuplicateKey {
            key: vec![0xc3, 0xa9],
        };
        assert!(duplicate.to_string().contains("c3a9"));
    }
}

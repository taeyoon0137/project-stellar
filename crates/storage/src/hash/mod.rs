//! content hash (V.1.5, ADR-0004).
//!
//! chunk payload·manifest를 주소화하는 `ContentHash` newtype과 문자열 형식을
//! 제공합니다. 문자열 형식은 `sha256:<64자리 소문자 16진수>`로 고정합니다.

use std::fmt;
use std::str::FromStr;

/// SHA-256 구현 (FIPS 180-4).
mod sha256;

/// content hash 알고리즘 registry (ADR-0004).
///
/// 새 알고리즘은 새 ADR로만 추가합니다. 문자열 prefix가 데이터에 남으므로
/// 이름은 안정적으로 유지되어야 합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HashAlgorithm {
    /// SHA-256 (FIPS 180-4). 문자열 prefix `sha256`.
    Sha256,
}

impl HashAlgorithm {
    /// 문자열 형식에 쓰는 안정 이름.
    pub fn name(self) -> &'static str {
        match self {
            HashAlgorithm::Sha256 => "sha256",
        }
    }

    /// digest 길이(byte).
    pub fn digest_len(self) -> usize {
        match self {
            HashAlgorithm::Sha256 => 32,
        }
    }
}

/// `ContentHash` 문자열 파싱 실패.
///
/// 입력은 신뢰되지 않은 파일 내용일 수 있습니다(`design/current`, envelope·pointer hash 문자열).
/// 그래서 [`fmt::Display`]·[`fmt::Debug`] 출력에는 입력 원문을 넣지 않습니다(보안 감사 STOR-6:
/// 알 수 없는 알고리즘 이름 자리에 비밀 값 한 줄이 들어와도 로그·UI에 남지 않음).
#[derive(Clone, PartialEq, Eq)]
pub enum ParseContentHashError {
    /// `<algorithm>:<hex>` 형태가 아닙니다.
    MissingSeparator,
    /// 알 수 없는 알고리즘 이름입니다.
    UnknownAlgorithm {
        /// 입력에 있던 알고리즘 이름. 오류 출력(`Display`·`Debug`)은 이 값 대신 byte 길이만
        /// 보여 줍니다.
        name: String,
    },
    /// hex 길이가 알고리즘 digest 길이와 다릅니다.
    InvalidLength {
        /// 기대 hex 길이.
        expected: usize,
        /// 실제 hex 길이.
        actual: usize,
    },
    /// 소문자 16진수가 아닌 문자가 있습니다.
    InvalidHexDigit {
        /// 위반 문자.
        found: char,
    },
}

impl fmt::Display for ParseContentHashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseContentHashError::MissingSeparator => {
                f.write_str("content hash는 '<algorithm>:<hex>' 형태여야 합니다")
            }
            ParseContentHashError::UnknownAlgorithm { name } => {
                write!(
                    f,
                    "알 수 없는 hash 알고리즘입니다(sha256만 지원, 입력 이름 {} bytes)",
                    name.len()
                )
            }
            ParseContentHashError::InvalidLength { expected, actual } => {
                write!(
                    f,
                    "hex 길이가 잘못되었습니다: 기대 {expected}, 실제 {actual}"
                )
            }
            ParseContentHashError::InvalidHexDigit { found } => {
                write!(f, "소문자 16진수가 아닌 문자입니다: {found:?}")
            }
        }
    }
}

impl fmt::Debug for ParseContentHashError {
    /// derive와 같은 모양이지만 알 수 없는 알고리즘 이름은 길이로만 보여 줍니다(원문 비노출).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseContentHashError::MissingSeparator => f.write_str("MissingSeparator"),
            ParseContentHashError::UnknownAlgorithm { name } => f
                .debug_struct("UnknownAlgorithm")
                .field("name", &format_args!("<생략, {} bytes>", name.len()))
                .finish(),
            ParseContentHashError::InvalidLength { expected, actual } => f
                .debug_struct("InvalidLength")
                .field("expected", expected)
                .field("actual", actual)
                .finish(),
            ParseContentHashError::InvalidHexDigit { found } => f
                .debug_struct("InvalidHexDigit")
                .field("found", found)
                .finish(),
        }
    }
}

impl std::error::Error for ParseContentHashError {}

/// content hash newtype (ADR-0004).
///
/// 알고리즘 tag + 32바이트 digest. 문자열 형식은
/// `sha256:<64자리 소문자 16진수>`(총 71자)입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash {
    algorithm: HashAlgorithm,
    digest: [u8; 32],
}

impl ContentHash {
    /// 현재 canonical 알고리즘(SHA-256)으로 입력의 content hash를 계산합니다.
    pub fn compute(input: &[u8]) -> ContentHash {
        ContentHash {
            algorithm: HashAlgorithm::Sha256,
            digest: sha256::sha256(input),
        }
    }

    /// 알고리즘.
    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    /// digest bytes.
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// digest의 소문자 16진수(알고리즘 prefix 없음).
    pub fn digest_hex(&self) -> String {
        self.digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algorithm.name(), self.digest_hex())
    }
}

impl FromStr for ContentHash {
    type Err = ParseContentHashError;

    fn from_str(s: &str) -> Result<ContentHash, ParseContentHashError> {
        let (name, hex) = s
            .split_once(':')
            .ok_or(ParseContentHashError::MissingSeparator)?;
        let algorithm = match name {
            "sha256" => HashAlgorithm::Sha256,
            other => {
                return Err(ParseContentHashError::UnknownAlgorithm {
                    name: other.to_string(),
                })
            }
        };
        let expected = algorithm.digest_len() * 2;
        if hex.len() != expected {
            return Err(ParseContentHashError::InvalidLength {
                expected,
                actual: hex.len(),
            });
        }
        // byte 길이가 맞아도 다중 byte 문자가 섞이면 2 byte slice가 문자 경계를 자를 수
        // 있으므로(XVI.6.2 fuzz 발견), slice 전에 모든 문자를 앞에서부터 검사합니다.
        if let Some(found) = hex
            .chars()
            .find(|c| !(c.is_ascii_digit() || ('a'..='f').contains(c)))
        {
            return Err(ParseContentHashError::InvalidHexDigit { found });
        }
        let mut digest = [0u8; 32];
        for (i, byte) in digest.iter_mut().enumerate() {
            // 위 검사로 hex는 ASCII뿐이라 모든 byte 위치가 문자 경계입니다.
            let pair = &hex[i * 2..i * 2 + 2];
            *byte = u8::from_str_radix(pair, 16).expect("소문자 hex 검증 완료");
        }
        Ok(ContentHash { algorithm, digest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_matches_sha256_test_vector() {
        // FIPS 180-4 "abc" vector.
        let hash = ContentHash::compute(b"abc");
        assert_eq!(
            hash.to_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(hash.algorithm(), HashAlgorithm::Sha256);
    }

    #[test]
    fn string_round_trip() {
        let original = ContentHash::compute(b"round trip");
        let text = original.to_string();
        assert_eq!(text.len(), 71, "sha256: 7자 + hex 64자");
        let parsed: ContentHash = text.parse().unwrap();
        assert_eq!(parsed, original);
        assert_eq!(parsed.to_string(), text);
    }

    #[test]
    fn parse_rejects_missing_separator() {
        let err = "deadbeef".parse::<ContentHash>().unwrap_err();
        assert_eq!(err, ParseContentHashError::MissingSeparator);
    }

    #[test]
    fn parse_rejects_unknown_algorithm() {
        let hex64 = "0".repeat(64);
        let err = format!("blake3:{hex64}")
            .parse::<ContentHash>()
            .unwrap_err();
        assert_eq!(
            err,
            ParseContentHashError::UnknownAlgorithm {
                name: "blake3".to_string()
            }
        );
    }

    /// 보안 감사 STOR-6: 오류 출력에 알고리즘 자리의 입력 원문을 넣지 않습니다(길이만).
    #[test]
    fn unknown_algorithm_error_does_not_echo_input() {
        let secret = "tok_SECRETVALUE_0123456";
        let err = format!("{secret}:x").parse::<ContentHash>().unwrap_err();
        assert_eq!(
            err,
            ParseContentHashError::UnknownAlgorithm {
                name: secret.to_string()
            }
        );
        let display = err.to_string();
        let debug = format!("{err:?}");
        for text in [&display, &debug] {
            assert!(!text.contains("SECRET"), "원문 노출: {text}");
            assert!(
                text.contains(&secret.len().to_string()),
                "길이 표시: {text}"
            );
        }
        assert_eq!(debug, "UnknownAlgorithm { name: <생략, 23 bytes> }");
        // 다른 variant의 Debug는 derive와 같은 모양입니다.
        assert_eq!(
            format!("{:?}", ParseContentHashError::MissingSeparator),
            "MissingSeparator"
        );
        assert_eq!(
            format!(
                "{:?}",
                ParseContentHashError::InvalidLength {
                    expected: 64,
                    actual: 4
                }
            ),
            "InvalidLength { expected: 64, actual: 4 }"
        );
        assert_eq!(
            format!(
                "{:?}",
                ParseContentHashError::InvalidHexDigit { found: 'g' }
            ),
            "InvalidHexDigit { found: 'g' }"
        );
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = "sha256:abcd".parse::<ContentHash>().unwrap_err();
        assert_eq!(
            err,
            ParseContentHashError::InvalidLength {
                expected: 64,
                actual: 4
            }
        );
    }

    #[test]
    fn parse_rejects_uppercase_hex() {
        let mut hex = "a".repeat(63);
        hex.push('F');
        let err = format!("sha256:{hex}").parse::<ContentHash>().unwrap_err();
        assert_eq!(err, ParseContentHashError::InvalidHexDigit { found: 'F' });
    }

    #[test]
    fn parse_rejects_non_hex_character() {
        let mut hex = "a".repeat(63);
        hex.push('g');
        let err = format!("sha256:{hex}").parse::<ContentHash>().unwrap_err();
        assert_eq!(err, ParseContentHashError::InvalidHexDigit { found: 'g' });
    }

    #[test]
    fn parse_rejects_multibyte_character_without_panic() {
        // XVI.6.2 fuzz 발견 회귀: byte 길이는 64지만 2·3·4 byte 문자가 hex 자리에 있으면
        // 문자 경계를 자르지 않고 그 문자를 InvalidHexDigit로 보고해야 합니다.
        for c in ['é', '€', '😀'] {
            for prefix in 0..=(64 - c.len_utf8()) {
                let mut hex = "a".repeat(prefix);
                hex.push(c);
                hex.push_str(&"a".repeat(64 - prefix - c.len_utf8()));
                assert_eq!(hex.len(), 64);
                let err = format!("sha256:{hex}").parse::<ContentHash>().unwrap_err();
                assert_eq!(err, ParseContentHashError::InvalidHexDigit { found: c });
            }
        }
    }

    #[test]
    fn ordering_is_stable_for_map_keys() {
        let a = ContentHash::compute(b"a");
        let b = ContentHash::compute(b"b");
        // Ord는 (algorithm, digest bytes) 순서 — 결정론적이면 충분합니다.
        assert_ne!(a, b);
        assert_eq!(a.cmp(&b), a.digest().cmp(b.digest()));
    }
}
